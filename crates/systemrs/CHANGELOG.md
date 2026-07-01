# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/systemrs/systemrs/releases/tag/systemrs-v0.1.0) - 2026-07-01

### Added

- add bounded snapshot/restore functionality
- *(pdes)* implement Tier-1 PDES orchestration with boundary links and local links
- add examples to documentation for various components including Fifo, Signal, and Memory
- add fixed-point reverb pedal example and enable-gated counter functionality
- *(twin)* implement digital-twin layer with real-time pacing, external input, and journaling
- *(tlm1)* introduce analysis port, fifo, and triple for telemetry
- Introduce AtMemory and convenience sockets for simplified TLM interactions
- complete Milestone 2 with attribute storage, two-level platform, and integration tests
- Introduce module construction with #[module] proc-macro and Kernel typestate
- *(tlm2)* implement TLM-2.0 transport layer with memory target and socket management

### Other

- repoint repository/homepage URLs to systemrs/systemrs
- add crates.io release pipeline (release-plz, trusted publishing, semver gates)
- Add TLM utilities for bus master, interconnect, and IRQ handling
- Refactor SystemC interop and related architecture
- Refactor code structure for improved readability and maintainability
