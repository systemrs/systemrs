#!/usr/bin/env bash
# Format the entire workspace in place.
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"
cargo fmt --all "$@"
