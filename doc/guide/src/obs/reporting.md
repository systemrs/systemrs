# Reporting

When a model needs to say something — an informational note, a warning, a recoverable
error, an unrecoverable one — it uses the **reporting** layer. SystemRS keeps SystemC's
severity/action/verbosity model but replaces `sc_report`-as-thrown-exception with plain
Rust: a recoverable error is a `Result`, and only a fatal aborts.

## Severities and the free functions

The quick path is the free functions, one per severity:

- `report_info(msg_type, message)` and `report_warning(...)` print to standard error.
- `error(msg_type, message)` returns a `ReportError` — a recoverable error you propagate
  with `?`, *not* a print and *not* a panic.
- `report_fatal(msg_type, message)` aborts the process for unrecoverable invariant
  corruption.

This maps the four `Severity` levels (`Info`, `Warning`, `Error`, `Fatal`) to Rust's
error model: `Error` → `Result`, `Fatal` → abort.

## Configurable resolution

For models that want SystemC-faithful control, a `ReportHandler` resolves each report to
a set of *actions* (display, log, cache, throw, abort) by the strict SystemC precedence —
a per-(message-type, severity) override beats a per-severity override beats the golden
default table — and gates `Info` reports by a `Verbosity` ceiling. The default action
table preserves the free functions' behaviour exactly: `Info`/`Warning` display,
`Error` throws (no print), `Fatal` aborts.

You will mostly use the free functions; reach for `ReportHandler` when you need to
suppress, escalate, or re-route a category of reports.

> **Go deeper:** design report §3.12 (reporting), §6e (observability).
