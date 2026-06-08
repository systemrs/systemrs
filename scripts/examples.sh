#!/usr/bin/env bash
# Run example binaries. With no arguments, runs every example in
# `systemrs-examples`; otherwise runs only the examples named on the command
# line (e.g. `scripts/examples.sh counter dma`).
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

names=("$@")
if [[ ${#names[@]} -eq 0 ]]; then
    # Each `[[example]]` maps to crates/systemrs-examples/examples/<name>.rs,
    # so the file stems are exactly the example names. Globbing keeps this in
    # sync with the manifest automatically.
    for f in crates/systemrs-examples/examples/*.rs; do
        names+=("$(basename "$f" .rs)")
    done
fi

for ex in "${names[@]}"; do
    printf '\n==> cargo run --example %s\n' "$ex"
    cargo run -p systemrs-examples --example "$ex"
done
