---
name: clipdeck-parity
description: Use when changing ClipDeck shared classification, previews, store, settings, hooks parsing, sync, accelerator or autostart behavior against the Rust implementation, or editing spec fixtures. Do not use for platform-only UI changes.
---

# ClipDeck parity

Deliver the requested behavior with matching contract and Rust semantics and
fixture evidence. Work from the repository root and follow its AGENTS.md.

1. Read the affected section of `spec/behavior.md`, the matching file under
   `spec/fixtures/`, and its reference implementation in `spec/verify.mjs`.
   Fixtures take precedence over conflicting prose; report the conflict.
2. Trace the behavior in `apps/linux/crates/core/src/` and the fixture consumer
   `apps/linux/crates/core/tests/fixtures.rs`.
3. Decide whether the request fixes implementation drift or intentionally changes
   the contract. Preserve expectations for a drift fix. For a contract change,
   update prose, synthetic fixture cases, oracle when relevant, the Rust
   implementation and consumer. Check schemas/examples for serialized changes.
4. Prove the bug with a focused regression case where practical. Preserve exact
   type/content dedupe, stack exclusions, unknown settings, Unicode scalar
   truncation and v1 store/sync compatibility unless explicitly in scope.
5. Run `bash scripts/check.sh linux-core`, including the fixture oracle. If app
   wiring changes, use `bash scripts/check.sh linux` as well. Never use real history
   or execute configured hooks as test inputs.

Report the affected contract section and cases, Rust behavior and
checks actually completed. Stop after the
requested parity is established; record unrelated platform gaps separately.
