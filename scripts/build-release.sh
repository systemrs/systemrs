#!/usr/bin/env bash
# Compile the whole workspace in release mode, matching the CI build leg.
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"
cargo build --workspace --release "$@"
