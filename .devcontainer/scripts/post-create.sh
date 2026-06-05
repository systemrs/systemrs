#!/usr/bin/env bash
#
# Runs once, the first time the SystemRS devcontainer is created.
# Keep everything here idempotent and non-fatal — a failed optional step should
# not block the container from coming up.
set -euo pipefail

WORKSPACE="/workspaces/systemrs"

echo "==> SystemRS devcontainer: post-create"

# Devcontainers frequently see the bind-mounted repo as "dubious ownership".
git config --global --add safe.directory "${WORKSPACE}" || true

# Ensure the components the Rust skill's checks need are present (the image
# installs them, but a toolchain update could drop them).
rustup component add rustfmt clippy >/dev/null 2>&1 || true

# The cargo registry/git caches are backed by named volumes (see
# devcontainer.json). A named volume's ownership is fixed when it is first
# created and is NOT re-derived from the image on later rebuilds, so the
# Dockerfile's build-time chown only covers a first-ever launch. Reconcile it
# here (the dev user has passwordless sudo) so cargo can always write the caches
# even after a UID remap or a volume reused from an older image.
for cache in /opt/cargo/registry /opt/cargo/git; do
    if [ -d "${cache}" ] && [ "$(stat -c %U "${cache}")" != "$(id -un)" ]; then
        echo "==> Reconciling ownership of ${cache}"
        sudo chown -R "$(id -un):$(id -gn)" "${cache}"
    fi
done

# Warm the crate cache once the workspace has been initialised as a Cargo
# workspace (see doc/systemrs-design.md §10 for the planned 14-crate layout).
if [ -f "${WORKSPACE}/Cargo.toml" ]; then
    echo "==> Fetching crate dependencies (cargo fetch)"
    cargo fetch --manifest-path "${WORKSPACE}/Cargo.toml" || true
else
    echo "==> No Cargo.toml yet — the Cargo workspace has not been created."
    echo "    See doc/systemrs-design.md §10 for the crate structure to scaffold."
fi

# The cxx/SystemC co-simulation path (systemrs-ffi, 'cosim' feature) builds
# against a SystemC checkout vendored at external/systemc (gitignored).
if [ -d "${WORKSPACE}/external/systemc" ]; then
    echo "==> external/systemc present — SystemC interop ('cosim') is available."
else
    echo "==> external/systemc not found."
    echo "    The 'cosim' feature builds against a SystemC source tree at"
    echo "    external/systemc. Clone one there when you need co-simulation."
fi

echo "==> post-create complete"
