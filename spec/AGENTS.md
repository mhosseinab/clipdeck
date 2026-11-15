# Shared behavior contract

Read the affected section of `behavior.md`, its fixtures, the Rust
implementation, and its fixture consumer before changing expectations.
Fixtures win over prose when they disagree; surface that discrepancy explicitly.

`verify.rs` is an independent reference oracle, not runtime application code.
Do not import it into the app or change it to match an implementation bug.
Use synthetic, deterministic cases with descriptive IDs and explicit expected
values. Follow the nearest fixture shape and add meaningful boundary cases.

For intentional behavior changes, update prose, fixtures, oracle when relevant,
and the Rust consumer/tests together. Format changes also require checking
`schema/` and `examples/` and existing store/sync format compatibility.

Run `bash scripts/check.sh spec`, then the Rust fixture suite. Oracle success only proves fixture/reference consistency, not native
implementation parity or desktop integration.
