# Rust Service Monitor

![rust-service-monitor project banner](docs/project-banner.webp)

A Rust CLI for concurrent HTTP/HTTPS endpoint checks, response-header timings, terminal results, and JSON reports. 

## Build and run

Use a current stable Rust toolchain (including Cargo) and a native build toolchain. On Fedora, install the distro packages with:

```bash
sudo dnf install -y rust cargo gcc
```

From the project folder:

```bash
cargo build --release --locked
cp endpoints.example.json endpoints.local.json
# Edit endpoints.local.json to select services you run or have permission to check.
cargo run --release --locked -- endpoints.local.json
```

Initial builds need internet access to download dependencies. `Cargo.lock` is included to lock the dependency resolution. The example targets the Support Case Tracker and Docker Deployment Lab on localhost; they must actually be running on ports 8777 and 8088. If they are stopped, failures are expected and do not mean the monitor itself is broken.

## Configure endpoints

```json
{
  "endpoints": [
    {
      "name": "My service",
      "url": "http://127.0.0.1:8777/health",
      "expected_statuses": [200, 204]
    }
  ]
}
```

Endpoint names must be unique, 1–80 characters, without control characters. Configure 1–32 endpoints, with absolute HTTP/HTTPS URLs no longer than 2,048 bytes. `expected_statuses` defaults to `[200]`; configure 1–20 status codes. Unknown config fields are rejected to catch typos. Config input is capped at 1 MiB.

URLs containing embedded credentials, query strings, or fragments are rejected. Authentication headers, request bodies, arbitrary HTTP methods, certificate bypasses, and port scanning are not supported. GET requests can still have side effects on poorly designed services; choose appropriate health endpoints you own or are authorized to test.

## Checks and timing

```bash
mkdir -p reports
cargo run --release --locked -- endpoints.local.json \
  --timeout-seconds 5 --workers 4 --json reports/check-01.json
```

- Checks use GET and measure elapsed time to receiving response headers, including connection setup where applicable. Bodies are not consumed or retained by application code; timing does not measure a full content download.
- A bounded thread pool uses 1–8 workers (default 4). Results preserve configuration order regardless of completion order.
- Each request has a 1–60 second timeout (default 5). Connect timeout is capped at five seconds. OS DNS behavior can affect how quickly underlying resolver work stops; this is not a hard real-time deadline for every platform.
- TLS verification uses reqwest/rustls with its bundled root configuration. Self-signed or private-CA services may fail; no custom CA configuration is implemented.
- Redirects are not followed. A 3xx is successful only if explicitly listed as expected; the destination is not tested.
- Proxy environment settings are ignored (`no_proxy`); proxy-only networks can fail direct connections.
- Transport failures are reported generically as timeout, connection/TLS failure, or HTTP transport failure. The tool does not infer a root cause.

This is a **one-shot check**, not a daemon, a load test, a historical uptime service, or an alerting system. HTTP status checks cannot prove that all application features work. No retry loop, scheduler, dashboard, credential store, or automatic remediation is included.

## Reports and exit codes

JSON includes a schema version, batch-start Unix timestamp in milliseconds, ordered endpoint results, status or transport error, elapsed milliseconds, and pass/fail counts. Existing files are never overwritten; create the parent directory first. On Unix, new report files use mode `0600`. Windows permissions follow the containing directory's ACL. If a report write fails, endpoint results may already have been printed and the command exits 2.

| Exit | Meaning |
| --- | --- |
| 0 | All endpoints returned an expected status |
| 1 | One or more endpoint checks failed |
| 2 | Invalid arguments/configuration, client setup failure, or report-file failure |

Private hostnames, URLs, and endpoint names can reveal infrastructure details. Keep actual config in the ignored `endpoints.local.json`, other private files ending in `.local.json`, and reports under ignored `reports/`. Review outputs before sharing. Never commit credentials. The example names refer only to local portfolio apps; their availability is not asserted.

## Test

```bash
cargo test --locked
```

During preparation, all ten integration tests passed, Rust formatting checks passed, and Clippy completed with warnings treated as errors. A separate local HTTPS smoke check confirmed that an untrusted self-signed certificate was rejected.

Ten integration tests use temporary files and local TCP servers for expected/unexpected HTTP statuses, custom accepted statuses, redirect refusal, timeout behavior, concurrent result ordering, config validation/defaults, report serialization/overwrite protection, and CLI exit codes. They do not contact public services or validate behavior on every operating system. The GitHub Actions workflow repeats the test/build steps after publication; inspect its results before calling CI successful.

## Publish to GitHub

With your GitHub CLI signed in:

```bash
bash publish.sh
```

This creates **public** repository `cuzzaintit321/rust-service-monitor`, adds its description, commits source, tests, example config, lockfile, and CI workflow, then pushes. It does not publish a crate to crates.io. The script stops if a local Git checkout or remote repository exists; inspect partial publication before retrying. No software license has been chosen on your behalf.

## Design choices

Rust owns configuration validation, bounded concurrency, stable result ordering, reporting, and exit behavior. `reqwest` handles HTTP and rustls-based TLS; `serde` and `serde_json` handle typed configuration and report serialization. This is native Rust HTTP code rather than a wrapper around `curl` or Python.
