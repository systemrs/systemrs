#!/usr/bin/env bash
# Detect SemVer-incompatible changes to the public API of each publishable crate,
# comparing the working tree against the latest version published to crates.io
# (cargo-semver-checks). Crates that have no published baseline yet are skipped, so
# this is a no-op before the first release and becomes a real gate afterwards.
#
# release-plz also runs cargo-semver-checks while preparing the release PR; this
# standalone recipe gives the same feedback on every PR, before that PR exists.
# `systemrs-examples` is `publish = false`, so it is excluded automatically.
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"
cargo semver-checks --workspace "$@"
