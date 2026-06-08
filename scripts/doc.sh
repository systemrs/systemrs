#!/usr/bin/env bash
# Build the API documentation, including private items.
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"
cargo doc --no-deps --document-private-items "$@"
