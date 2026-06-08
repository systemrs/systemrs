#!/usr/bin/env bash
# Build the API documentation and open it in a browser.
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"
cargo doc --no-deps --document-private-items --open "$@"
