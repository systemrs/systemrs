# SystemRS Development Container
#
# SystemRS is a pure-Rust, host-based TLM simulation library (a TLM-only
# equivalent of SystemC). This image therefore provides:
#   - a Rust toolchain (current stable + the project MSRV) with rustfmt/clippy;
#   - the cargo quality tools the project's checks rely on (deny, audit, nextest);
#   - a C/C++ toolchain + CMake for the `cxx`-based SystemC interop
#     (`systemrs-ffi`, built against external/systemc under the `cosim` feature);
#   - Node.js + the Claude Code CLI.
#
# It deliberately does NOT carry FPGA/USB programming tools, embedded
# cross-compilation targets, or register-generation/datasheet tooling — none of
# which a host-only TLM simulator needs.
FROM ubuntu:24.04

# Avoid prompts during package installation.
ENV DEBIAN_FRONTEND=noninteractive

# Base build dependencies.
#   build-essential + cmake + pkg-config back the `cc`/`cxx` crates and let us
#   build the vendored SystemC reference (external/systemc) for co-simulation.
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    cmake \
    pkg-config \
    git \
    curl \
    ca-certificates \
    gnupg \
    make \
    sudo \
    locales \
    && rm -rf /var/lib/apt/lists/*

# Set up locale.
RUN locale-gen en_US.UTF-8
ENV LANG=en_US.UTF-8
ENV LANGUAGE=en_US:en
ENV LC_ALL=en_US.UTF-8

# Install Node.js 20.x LTS (required for the Claude Code CLI).
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/*

# Install Rust via rustup into a shared location that is on PATH for every user.
# A minimal profile keeps the image small; rustfmt + clippy are added explicitly
# because the project's build-verification sequence (see CLAUDE.md / the Rust
# skill) runs `cargo fmt` and `cargo clippy`.
ENV RUSTUP_HOME=/opt/rustup
ENV CARGO_HOME=/opt/cargo
ENV PATH="${CARGO_HOME}/bin:${PATH}"
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain stable \
    && rustup component add rustfmt clippy

# Keep the project MSRV available so `cargo +1.90 ...` reproduces the MSRV leg of
# CI locally (the workspace pins rust-version = "1.90"; CI also tests stable).
RUN rustup toolchain install 1.90 --profile minimal \
    && rustup component add --toolchain 1.90 rustfmt clippy

# Cargo quality tools used by the build-verification workflow:
#   cargo-deny  — license + advisory + duplicate-version gate (deny.toml)
#   cargo-audit — RustSec vulnerability scanner
#   cargo-nextest — fast test runner for the conformance/integration suites
RUN cargo install --locked cargo-deny cargo-audit cargo-nextest

# Install Claude Code CLI globally.
RUN npm install -g @anthropic-ai/claude-code

# Create non-root user for development (handle existing UID/GID 1000).
ARG USERNAME=vscode
ARG USER_UID=1000
ARG USER_GID=${USER_UID}

RUN if getent group ${USER_GID} > /dev/null 2>&1; then \
    existing_group=$(getent group ${USER_GID} | cut -d: -f1); \
    groupmod -n ${USERNAME} ${existing_group}; \
    else \
    groupadd --gid ${USER_GID} ${USERNAME}; \
    fi \
    && if getent passwd ${USER_UID} > /dev/null 2>&1; then \
    existing_user=$(getent passwd ${USER_UID} | cut -d: -f1); \
    usermod -l ${USERNAME} -d /home/${USERNAME} -m ${existing_user}; \
    else \
    useradd --uid ${USER_UID} --gid ${USER_GID} -m ${USERNAME}; \
    fi \
    && echo "${USERNAME} ALL=(ALL) NOPASSWD:ALL" > /etc/sudoers.d/${USERNAME} \
    && chmod 0440 /etc/sudoers.d/${USERNAME}

# Hand the Rust install to the dev user so it can add components and download
# crates without sudo. The registry/git caches live here and are backed by named
# volumes (see devcontainer.json) so downloads survive container rebuilds.
RUN mkdir -p /opt/cargo/registry /opt/cargo/git \
    && chown -R ${USERNAME}:${USERNAME} /opt/cargo /opt/rustup

# Store Claude Code config in the workspace so plans and memory persist across
# devcontainer rebuilds.
ENV CLAUDE_CONFIG_DIR=/workspaces/systemrs/.claude

# Final user switch + workdir (matches workspaceFolder in devcontainer.json).
USER ${USERNAME}
WORKDIR /workspaces/systemrs
