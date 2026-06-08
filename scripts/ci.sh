#!/usr/bin/env bash
# Run every CI quality check in the project's build-verification order (see
# CLAUDE.md): fmt --check -> clippy -> test -> build (release) -> examples ->
# doc -> deny -> audit.
#
# This is the single definition of "what CI does", shared by `just ci` and the
# GitHub Actions workflow. Each step delegates to its sibling script so there is
# exactly one place that owns each command.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$here/_common.sh"

run() {
    printf '\n\033[1m== %s ==\033[0m\n' "$1"
    shift
    "$@"
}

run "fmt --check"     "$here/fmt-check.sh"
run "clippy"          "$here/clippy.sh"
run "test"            "$here/test.sh"
run "build (release)" "$here/build-release.sh"
run "examples"        "$here/examples.sh"
run "doc"             "$here/doc.sh"
run "deny"            "$here/deny.sh"
run "audit"           "$here/audit.sh"

printf '\n\033[1;32mAll CI checks passed.\033[0m\n'
