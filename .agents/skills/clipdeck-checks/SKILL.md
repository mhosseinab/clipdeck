---
name: clipdeck-checks
description: Use when validating a ClipDeck change, choosing its Rust and fixture checks, or preparing an evidence-based handoff. Do not use for unrelated projects or dependency installation.
---

# ClipDeck checks

Select checks from the actual diff and acceptance criteria. Read AGENTS.md,
applicable nested instructions and `.github/workflows/ci.yml`; distinguish the
task's edits from pre-existing work. Run from the repository root:

| Scope | Command | Evidence |
| --- | --- | --- |
| Contract/fixtures only | `bash scripts/check.sh spec` | Oracle consistency |
| Rust core | `bash scripts/check.sh linux-core` | Oracle + Rust core tests |
| Linux app/integration | `bash scripts/check.sh linux` | Oracle + workspace tests + release build |
| Local candidate | `bash scripts/package.sh` | Tests + build + inspected packages + checksums |

For domain behavior, the Rust fixture suite and oracle must both pass.
Oracle success alone does not prove the runtime implementation.

- Inspect missing tool, system-library, dependency-cache and network errors;
  report them accurately instead of installing packages or altering the project.
- Use focused regression tests beyond existing checks only when needed to prove
  the requested behavior. For docs/config-only changes, validate the changed
  artifacts and discovery; a full native rebuild is unnecessary.
- Do not run a formatter across unrelated files. Check whitespace with
  `git diff --check`; inspect untracked setup files as well.
- Never launch ClipDeck, use live clipboard history, run configured hooks, change
  autostart or grant OS permissions without explicit interactive-test scope.
- For UI, tray, focus, paste and shortcut changes, state what requires desktop
  verification. Compilation does not establish those behaviors.

Finish with exact commands and outcomes, unavailable checks and any relevant
pre-existing failures. Do not describe skipped or blocked checks as passing.
