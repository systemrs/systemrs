# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/systemrs/systemrs/releases/tag/systemrs-core-v0.1.0) - 2026-07-01

### Added

- add examples to documentation for various components including Fifo, Signal, and Memory
- *(tlm1)* introduce analysis port, fifo, and triple for telemetry
- complete Milestone 2 with attribute storage, two-level platform, and integration tests
- enhance two-phase binding with resolved interface retrieval and error handling
- Introduce module construction with #[module] proc-macro and Kernel typestate
- *(elaboration)* implement elaboration driver and lifecycle callbacks for SystemRS
- *(tlm2)* implement TLM-2.0 transport layer with memory target and socket management

### Other

- repoint repository/homepage URLs to systemrs/systemrs
- add crates.io release pipeline (release-plz, trusted publishing, semver gates)
- Refactor code structure for improved readability and maintainability
