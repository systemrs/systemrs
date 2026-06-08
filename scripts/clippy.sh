#!/usr/bin/env bash
# Lint every target across the workspace with warnings promoted to errors.
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"
cargo clippy --workspace --all-targets "$@" -- -D warnings
