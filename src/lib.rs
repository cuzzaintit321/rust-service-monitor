use reqwest::{blocking::Client, redirect::Policy, Url};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn default_statuses() -> Vec<u16> {
    vec![200]
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub name: String,
    pub url: String,
    #[serde(default = "default_statuses")]
    pub expected_statuses: Vec<u16>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub endpoints: Vec<Endpoint>,
}

pub fn validate(config: &Config) -> Result<(), String> {
    if config.endpoints.is_empty() || config.endpoints.len() > 32 {
        return Err("Configure between 1 and 32 endpoints".into());
    }
    let mut names = std::collections::HashSet::new();
    for endpoint in &config.endpoints {
        if endpoint.name.trim().is_empty()
            || endpoint.name.chars().count() > 80
            || endpoint.name.chars().any(char::is_control)
        {
            return Err(
                "Endpoint names must contain 1–80 characters without control characters".into(),
            );
        }
        if !names.insert(endpoint.name.clone()) {
            return Err("Endpoint names must be unique".into());
        }
        if endpoint.url.len() > 2048 {
            return Err("Endpoint URL is too long".into());
        }
        let url = Url::parse(&endpoint.url).map_err(|_| "Invalid endpoint URL")?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err("Only absolute HTTP and HTTPS URLs are supported".into());
        }
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(
                "Do not use credentials, query strings, or fragments in endpoint URLs".into(),
            );
        }
        if endpoint.url.chars().any(char::is_control) {
            return Err("Control characters are not allowed in URLs".into());
        }
        if endpoint.expected_statuses.is_empty()
            || endpoint.expected_statuses.len() > 20
            || endpoint
                .expected_statuses
                .iter()
                .any(|status| !(100..=599).contains(status))
        {
            return Err(
                "Expected statuses must contain 1–20 HTTP status codes between 100 and 599".into(),
            );
        }
    }
    Ok(())
}

pub fn load_config(path: &Path) -> Result<Config, String> {
    // Read through a bounded reader as the file may grow after metadata checks.
    use std::io::Read;
    let file = fs::File::open(path).map_err(|error| format!("Cannot open config: {error}"))?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Config must be a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(1_048_577)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1_048_576 {
        return Err("Config must not exceed 1 MiB".into());
    }
    let config: Config =
        serde_json::from_slice(&bytes).map_err(|error| format!("Invalid config JSON: {error}"))?;
    validate(&config)?;
    Ok(config)
}

#[derive(Debug, Serialize)]
pub struct CheckResult {
    pub name: String,
    pub url: String,
    pub ok: bool,
    pub status: Option<u16>,
    pub elapsed_ms: u64,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Summary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub schema_version: u8,
    pub checked_at_unix_ms: u64,
    pub summary: Summary,
    pub results: Vec<CheckResult>,
}

fn check(client: &Client, endpoint: &Endpoint) -> CheckResult {
    let start = Instant::now();
    // Only response headers are measured. Do not download/retain response bodies.
    let response = client.get(&endpoint.url).send();
    let elapsed_ms = start.elapsed().as_millis() as u64;
    let (status, ok, error) = match response {
        Ok(response) => {
            let status = response.status().as_u16();
            let ok = endpoint.expected_statuses.contains(&status);
            (
                Some(status),
                ok,
                if ok {
                    None
                } else {
                    Some("Unexpected HTTP status".into())
                },
            )
        }
        Err(error) => (
            None,
            false,
            Some(
                if error.is_timeout() {
                    "Request timed out"
                } else if error.is_connect() {
                    "Connection or TLS failure"
                } else {
                    "HTTP transport failure"
                }
                .into(),
            ),
        ),
    };
    CheckResult {
        name: endpoint.name.clone(),
        url: endpoint.url.clone(),
        ok,
        status,
        elapsed_ms,
        error,
    }
}

pub fn run(config: &Config, timeout_seconds: u64, workers: usize) -> Result<Report, String> {
    validate(config)?;
    if !(1..=60).contains(&timeout_seconds) {
        return Err("Timeout must be 1–60 seconds".into());
    }
    if !(1..=8).contains(&workers) {
        return Err("Workers must be 1–8".into());
    }
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_seconds))
        .connect_timeout(Duration::from_secs(timeout_seconds.min(5)))
        .redirect(Policy::none())
        .no_proxy()
        .user_agent("rust-service-monitor/0.1.0")
        .build()
        .map_err(|e| format!("Cannot build HTTP client: {e}"))?;
    let cursor = AtomicUsize::new(0);
    let output = Mutex::new(Vec::new());
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis() as u64;
    thread::scope(|scope| {
        for _ in 0..workers.min(config.endpoints.len()) {
            let client = &client;
            let cursor = &cursor;
            let output = &output;
            scope.spawn(move || loop {
                let index = cursor.fetch_add(1, Ordering::Relaxed);
                if index >= config.endpoints.len() {
                    break;
                }
                let result = check(client, &config.endpoints[index]);
                output
                    .lock()
                    .expect("Result lock poisoned")
                    .push((index, result));
            });
        }
    });
    let mut output = output.into_inner().map_err(|_| "Result lock poisoned")?;
    output.sort_by_key(|(index, _)| *index);
    let results: Vec<_> = output.into_iter().map(|(_, value)| value).collect();
    let passed = results.iter().filter(|item| item.ok).count();
    Ok(Report {
        schema_version: 1,
        checked_at_unix_ms: timestamp,
        summary: Summary {
            total: results.len(),
            passed,
            failed: results.len() - passed,
        },
        results,
    })
}

pub fn write_report(path: &Path, report: &Report) -> Result<(), String> {
    let data = serde_json::to_vec_pretty(report).map_err(|e| e.to_string())?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|e| format!("Cannot create report (existing files are not overwritten): {e}"))?;
    if let Err(error) = file.write_all(&data).and_then(|_| file.write_all(b"\n")) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(format!("Report write failed: {error}"));
    }
    Ok(())
}
