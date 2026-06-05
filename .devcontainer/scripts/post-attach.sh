#!/usr/bin/env bash
#
# Runs every time a session attaches to the SystemRS devcontainer.
# Keep this fast and side-effect-free — it is on the interactive attach path.
set -euo pipefail

echo "SystemRS dev environment"
echo "  rustc  : $(rustc --version 2>/dev/null || echo 'not found')"
echo "  cargo  : $(cargo --version 2>/dev/null || echo 'not found')"
echo "  clippy : $(cargo-clippy --version 2>/dev/null || echo 'not found')"
# Only query the MSRV toolchain if it is already installed — naming `+1.90`
# otherwise makes rustup download a full toolchain, which would not be
# "side-effect-free" on the interactive attach path.
if rustup toolchain list 2>/dev/null | grep -q '^1\.90'; then
    echo "  MSRV   : $(rustc +1.90 --version 2>/dev/null || echo '1.90 toolchain present')"
else
    echo "  MSRV   : 1.90 toolchain not installed"
fi
