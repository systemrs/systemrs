#!/usr/bin/env bash
# Verify formatting without writing changes; fails if anything is unformatted.
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"
cargo fmt --all --check "$@"
