#!/bin/bash
# The full check before a merge: format, lints, tests, and no API key anywhere in the tree.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all --check
systemd-run --user --scope -q -p MemoryMax=16G cargo clippy --workspace --all-targets -j 4 -- -D warnings
systemd-run --user --scope -q -p MemoryMax=16G cargo test --workspace -j 4
if git grep -nE 'cb1_[A-Za-z0-9_]{6,}|key=[A-Za-z0-9_]{16,}' -- . ':!scripts/check-all.sh'; then
    echo "Looks like an API key in the repository." >&2
    exit 1
fi
echo "All checks passed."
