#!/usr/bin/env bash
# Run the whole workspace test suite (unit, integration, and doctests).
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"
cargo test --workspace "$@"
