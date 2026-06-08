#!/usr/bin/env bash
# Build the SystemRS user guide (mdBook) — the hard documentation gate.
#
# `mdbook build` renders the book, checks every intra-book link, and resolves every
# {{#include ...:ANCHOR}} — so a moved/renamed example or a bad anchor fails here. We
# build the example crates first so a broken include *source* surfaces as a compile
# error rather than a silently stale include.
#
# We deliberately do NOT run `mdbook test`: it invokes rustdoc per code block but
# cannot link the `systemrs` facade crate (no `--extern` passthrough, and
# target/debug/deps carries many hashed rlibs), so it would fail on every
# prelude-using snippet for tooling reasons, not real ones. Code stays honest without
# it: the large listings are `{{#include}}`d verbatim from the example sources that
# `cargo test` + `scripts/examples.sh` already compile and run, and the small inline
# snippets mirror the facade's verified rustdoc doctests (`cargo test --doc`).
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

cargo build -p systemrs -p systemrs-examples

mdbook build doc/guide "$@"
