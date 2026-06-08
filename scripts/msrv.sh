#!/usr/bin/env bash
# Build and test the workspace on the Minimum Supported Rust Version (MSRV).
#
# The MSRV is read from `rust-version` in the workspace Cargo.toml, so this stays
# correct automatically when that floor is raised — there is no second copy of the
# version to keep in sync. The toolchain is installed via rustup if it is missing
# (rustup does not auto-install for the `cargo +<version>` override).
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

# Read the declared MSRV (e.g. 1.90) from [workspace.package].rust-version.
msrv="$(sed -n 's/^[[:space:]]*rust-version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' Cargo.toml | head -n1)"
if [[ -z "$msrv" ]]; then
    echo "error: could not determine the MSRV from rust-version in Cargo.toml" >&2
    exit 1
fi

# Ensure the toolchain is available before invoking `cargo +<msrv>`.
if ! rustup toolchain list | grep -qE "^${msrv//./\\.}-"; then
    echo "==> installing Rust ${msrv} toolchain"
    rustup toolchain install "${msrv}" --profile minimal
fi

echo "==> building and testing on Rust ${msrv} (MSRV)"
cargo "+${msrv}" build --workspace
cargo "+${msrv}" test --workspace
