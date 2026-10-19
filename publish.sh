#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "$0")"
gh auth status
gh auth setup-git
if [[ -e .git ]]; then echo 'Git checkout exists; review before publishing manually.'; exit 1; fi
if gh repo view cuzzaintit321/rust-service-monitor >/dev/null 2>&1; then
    echo 'Repository exists; review before publishing manually.'; exit 1
fi
git init -b main
git add .
git -c user.name='Jason Edwards' \
    -c user.email='228125969+cuzzaintit321@users.noreply.github.com' \
    commit -m 'Add Rust endpoint monitor with bounded concurrency, JSON reports, tests, and CI'
gh repo create cuzzaintit321/rust-service-monitor --public --source . --remote origin --push \
    --description 'Concurrent Rust HTTP/HTTPS service monitor with timeouts, response timings, expected status checks, and JSON reports.'
