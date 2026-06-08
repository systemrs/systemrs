# SystemRS task runner — see https://just.systems
#
# Every recipe is a thin wrapper around a script in `scripts/`, so the exact same
# commands run locally (`just <recipe>`) and in CI (the GitHub Actions workflow
# calls `just ci`, which calls the same scripts). A green `just ci` locally means
# a green CI. Run `just` with no arguments to list everything.
#
# Most recipes forward extra arguments to the underlying cargo command, e.g.
# `just test delta_order` or `just clippy -p systemrs-kernel`.

# List the available recipes (default).
default:
    @just --list

# --- individual CI actions (each delegates to scripts/) ---

# Format the whole workspace in place.
fmt *args:
    scripts/fmt.sh {{args}}

# Check formatting without modifying files (CI gate).
fmt-check *args:
    scripts/fmt-check.sh {{args}}

# Lint all targets; warnings are errors.
clippy *args:
    scripts/clippy.sh {{args}}

# Compile the whole workspace and all targets (debug).
build *args:
    scripts/build.sh {{args}}

# Compile the whole workspace in release mode (as CI does).
build-release *args:
    scripts/build-release.sh {{args}}

# Run the whole test suite.
test *args:
    scripts/test.sh {{args}}

# Build and test on the Minimum Supported Rust Version (installs it if needed).
msrv:
    scripts/msrv.sh

# Run the example binaries (all of them, or only those named).
examples *names:
    scripts/examples.sh {{names}}

# Build the API docs (private items included).
doc *args:
    scripts/doc.sh {{args}}

# Build the API docs and open them in a browser.
open-docs *args:
    scripts/open-docs.sh {{args}}

# Check dependency licenses, advisories, and bans (cargo-deny).
deny *args:
    scripts/deny.sh {{args}}

# Audit dependencies for known security advisories (cargo-audit).
audit *args:
    scripts/audit.sh {{args}}

# --- aggregate ---

# Run every CI quality check, in the project's build-verification order.
ci:
    scripts/ci.sh

alias check := ci
