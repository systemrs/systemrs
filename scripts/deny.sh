#!/usr/bin/env bash
# Check dependency licenses, advisories, bans, and sources (cargo-deny).
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"
cargo deny check "$@"
