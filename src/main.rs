use rust_service_monitor::{load_config, run, write_report};
use std::env;
use std::path::PathBuf;

fn execute(args: Vec<String>) -> Result<i32, String> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("Rust Service Monitor\nUsage: rust-service-monitor CONFIG.json [--timeout-seconds 5] [--workers 4] [--json NEW_FILE]\nExit: 0 all checks passed; 1 failed endpoint checks; 2 configuration/report error.");
        return Ok(0);
    }
    if args.is_empty() || args[0].starts_with('-') {
        return Err("Provide a config filename; use --help".into());
    }
    let config_path = PathBuf::from(&args[0]);
    let mut timeout = 5_u64;
    let mut workers = 4_usize;
    let mut report_path = None;
    let mut index = 1;
    let mut seen = std::collections::HashSet::new();
    while index < args.len() {
        let option = &args[index];
        if !matches!(
            option.as_str(),
            "--timeout-seconds" | "--workers" | "--json"
        ) {
            return Err(format!("Unknown option: {option}"));
        }
        if !seen.insert(option) {
            return Err(format!("Duplicate option: {option}"));
        }
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("Missing value for {option}"))?;
        match option.as_str() {
            "--timeout-seconds" => {
                timeout = value.parse().map_err(|_| "Timeout must be an integer")?
            }
            "--workers" => workers = value.parse().map_err(|_| "Workers must be an integer")?,
            "--json" => {
                if value.starts_with("--") {
                    return Err("Missing report filename".into());
                }
                report_path = Some(PathBuf::from(value));
            }
            _ => unreachable!(),
        }
        index += 2;
    }
    let config = load_config(&config_path)?;
    let report = run(&config, timeout, workers)?;
    println!("RUST SERVICE MONITOR");
    for result in &report.results {
        let status = result
            .status
            .map_or_else(|| "---".to_string(), |value| value.to_string());
        println!(
            "[{}] {} | HTTP {} | {} ms{}",
            if result.ok { "PASS" } else { "FAIL" },
            result.name,
            status,
            result.elapsed_ms,
            result
                .error
                .as_ref()
                .map_or(String::new(), |e| format!(" | {e}"))
        );
    }
    println!(
        "{} checked | {} passed | {} failed",
        report.summary.total, report.summary.passed, report.summary.failed
    );
    if let Some(path) = report_path {
        write_report(&path, &report)?;
        println!("JSON report saved.");
    }
    Ok(if report.summary.failed == 0 { 0 } else { 1 })
}

fn main() {
    let code = match execute(env::args().skip(1).collect()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("Rust Service Monitor: {error}");
            2
        }
    };
    std::process::exit(code);
}
