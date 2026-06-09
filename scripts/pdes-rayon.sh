#!/usr/bin/env bash
# Exercise the optional rayon-backed Tier-1 PDES parallel backend (systemrs-pdes
# `rayon` feature). The default `just ci` build is rayon-free (and `unsafe`-free); this
# step lints the single audited `unsafe impl Send for Region`, then runs the determinism
# tests with the parallel backend ON — proving the rayon-parallel Tier-1 run produces a
# trace bit-identical to the sequential Tier-0 reference (design §8a; plan-m7 E5).
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

cargo clippy -p systemrs-pdes --all-targets --features rayon -- -D warnings
cargo test -p systemrs-examples --test pdes_determinism --features systemrs/pdes-rayon
