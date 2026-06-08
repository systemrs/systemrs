#!/usr/bin/env bash
# Shared setup for the SystemRS task scripts in this folder.
#
# Every script sources this first so it (a) aborts on the first error, unset
# variable, or failed pipe and (b) runs from the repository root. That makes the
# scripts behave identically whether invoked via `just <recipe>`, from a GitHub
# Actions step, or by hand from any directory.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
