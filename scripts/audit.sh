#!/usr/bin/env bash
# Audit the dependency tree for known security advisories (cargo-audit).
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"
cargo audit "$@"
