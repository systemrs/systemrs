#!/usr/bin/env bash
# Compile the whole workspace, including examples, tests, and benches (debug).
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"
cargo build --workspace --all-targets "$@"
