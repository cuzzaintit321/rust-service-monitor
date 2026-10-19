use rust_service_monitor::{load_config, run, validate, write_report, Config, Endpoint};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "rust-monitor-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn endpoint(name: &str, url: &str) -> Endpoint {
    Endpoint {
        name: name.into(),
        url: url.into(),
        expected_statuses: vec![200],
    }
}

fn fixture(response: &'static str, delay_ms: u64) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut buffer = [0; 4096];
        let _ = stream.read(&mut buffer);
        thread::sleep(Duration::from_millis(delay_ms));
        let _ = stream.write_all(response.as_bytes());
    });
    (format!("http://{address}/health"), handle)
}

#[test]
fn successful_http_check_records_status() {
    let (url, server) = fixture(
        "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        0,
    );
    let result = run(
        &Config {
            endpoints: vec![endpoint("healthy", &url)],
        },
        2,
        1,
    )
    .unwrap();
    server.join().unwrap();
    assert_eq!(result.summary.passed, 1);
    assert_eq!(result.results[0].status, Some(200));
}

#[test]
fn unexpected_http_status_is_a_failure() {
    let (url, server) = fixture(
        "HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        0,
    );
    let result = run(
        &Config {
            endpoints: vec![endpoint("unhealthy", &url)],
        },
        2,
        1,
    )
    .unwrap();
    server.join().unwrap();
    assert_eq!(result.summary.failed, 1);
    assert_eq!(result.results[0].status, Some(503));
}

#[test]
fn expected_status_can_be_customized() {
    let (url, server) = fixture("HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n", 0);
    let mut item = endpoint("no-content", &url);
    item.expected_statuses = vec![200, 204];
    let result = run(
        &Config {
            endpoints: vec![item],
        },
        2,
        1,
    )
    .unwrap();
    server.join().unwrap();
    assert_eq!(result.summary.failed, 0);
}

#[test]
fn redirects_are_not_followed() {
    let (url, server) = fixture("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/elsewhere\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", 0);
    let result = run(
        &Config {
            endpoints: vec![endpoint("redirect", &url)],
        },
        2,
        1,
    )
    .unwrap();
    server.join().unwrap();
    assert_eq!(result.results[0].status, Some(302));
    assert!(!result.results[0].ok);
}

#[test]
fn slow_response_times_out() {
    let (url, server) = fixture("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n", 1400);
    let result = run(
        &Config {
            endpoints: vec![endpoint("slow", &url)],
        },
        1,
        1,
    )
    .unwrap();
    server.join().unwrap();
    assert_eq!(
        result.results[0].error.as_deref(),
        Some("Request timed out")
    );
    assert_eq!(result.summary.failed, 1);
}

#[test]
fn concurrent_completion_keeps_configuration_order() {
    let (slow, first) = fixture("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n", 150);
    let (fast, second) = fixture("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n", 0);
    let result = run(
        &Config {
            endpoints: vec![endpoint("first", &slow), endpoint("second", &fast)],
        },
        2,
        2,
    )
    .unwrap();
    first.join().unwrap();
    second.join().unwrap();
    assert_eq!(
        result
            .results
            .iter()
            .map(|x| x.name.as_str())
            .collect::<Vec<_>>(),
        vec!["first", "second"]
    );
    assert_eq!(result.summary.passed, 2);
}

#[test]
fn validation_rejects_credentials_queries_and_duplicate_names() {
    for url in [
        "ftp://example.com/",
        "https://user:pass@example.com/",
        "https://example.com/?token=secret",
        "https://example.com/#fragment",
    ] {
        assert!(validate(&Config {
            endpoints: vec![endpoint("unsafe", url)]
        })
        .is_err());
    }
    assert!(validate(&Config {
        endpoints: vec![
            endpoint("same", "http://localhost/"),
            endpoint("same", "http://localhost/")
        ]
    })
    .is_err());
    assert!(run(
        &Config {
            endpoints: vec![endpoint("test", "http://localhost/")]
        },
        0,
        1
    )
    .is_err());
    assert!(run(
        &Config {
            endpoints: vec![endpoint("test", "http://localhost/")]
        },
        1,
        9
    )
    .is_err());
}

#[test]
fn config_defaults_and_unknown_fields_are_checked() {
    let dir = TempDir::new();
    let path = dir.0.join("config.json");
    fs::write(
        &path,
        r#"{"endpoints":[{"name":"demo","url":"http://localhost/"}]}"#,
    )
    .unwrap();
    assert_eq!(
        load_config(&path).unwrap().endpoints[0].expected_statuses,
        vec![200]
    );
    fs::write(
        &path,
        r#"{"endpoints":[{"name":"demo","url":"http://localhost/","misspelled":true}]}"#,
    )
    .unwrap();
    assert!(load_config(&path).is_err());
}

#[test]
fn report_serialization_and_overwrite_protection() {
    let (url, server) = fixture("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n", 0);
    let report = run(
        &Config {
            endpoints: vec![endpoint("demo", &url)],
        },
        2,
        1,
    )
    .unwrap();
    server.join().unwrap();
    let dir = TempDir::new();
    let path = dir.0.join("report.json");
    write_report(&path, &report).unwrap();
    let original = fs::read(&path).unwrap();
    let data: serde_json::Value = serde_json::from_slice(&original).unwrap();
    assert_eq!(data["summary"]["passed"], 1);
    assert!(write_report(&path, &report).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn cli_failure_status_and_json_report() {
    let (url, server) = fixture("HTTP/1.1 500 Error\r\nContent-Length: 0\r\n\r\n", 0);
    let dir = TempDir::new();
    let config = dir.0.join("config.json");
    let report = dir.0.join("report.json");
    fs::write(
        &config,
        serde_json::json!({"endpoints":[{"name":"cli-demo","url":url}]}).to_string(),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rust-service-monitor"))
        .arg(&config)
        .arg("--json")
        .arg(&report)
        .output()
        .unwrap();
    server.join().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(report.exists());
    let invalid = Command::new(env!("CARGO_BIN_EXE_rust-service-monitor"))
        .arg(&config)
        .args(["--workers", "0"])
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
}
