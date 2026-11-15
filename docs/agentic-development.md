# Agentic development in ClipDeck

This setup is scoped to the repository. It adds no packages, background services,
credentials, automatic hooks, model selection, or changes to global settings.

## Start a task

Open the repo in Codex and describe an outcome, acceptance criteria and scope.
For example: "Fix duplicate clipboard items in the Rust core. Preserve the
shared contract and add a regression case. Leave the Linux UI unchanged."

`AGENTS.md` holds everyday rules; nested files cover Linux and the
spec. Agents should inspect existing edits before changing files and finish
with evidence from checks appropriate to the task.

## Reusable skills

Repository skills are discovered under `.agents/skills/`. Open a new project
chat after changing skill metadata so the client can refresh discovery.

- `$clipdeck-parity`: trace a shared behavior change through contract, fixtures,
  reference oracle and the Rust implementation.
- `$clipdeck-checks`: select relevant checks, run them and report unsupported
  hosts, missing tools or integration evidence still needed.

Example: "Use $clipdeck-parity to add a boundary case for Unicode title
truncation and verify the Rust core against the existing contract."

## Optional agents

`.codex/config.toml` registers `clipdeck_explorer` (locate implementations,
contracts and tests) and `clipdeck_reviewer` (inspect a completed diff for
correctness and parity regressions). Their role files instruct them to inspect
and report without editing. They inherit the session's model and permissions;
role instructions are not an independent security boundary.

Delegation requires explicit permission for the task. Example: "You may use
parallel agents: have the explorer trace the contract and Rust paths while you
implement the fix, then ask the reviewer to inspect the diff."

Support for custom roles depends on the Codex client and project trust. If a
client does not expose the roles, perform the same workflows in the main chat;
do not weaken permissions or enable features automatically to compensate.

## Checks

Run these from the repo root, or pass the script's absolute path from elsewhere:

```bash
bash scripts/check.sh spec
bash scripts/check.sh linux-core
bash scripts/check.sh linux
bash scripts/package.sh  # local artifacts only; no publication
```

Each check scope runs the fixture oracle. `linux-core` adds Rust core tests;
`linux` adds workspace tests and a release build. Packaging also validates the
Debian payload, desktop entry, tarball and checksums. No command publishes,
installs a package or launches ClipDeck. Native toolchains, cargo-deb and system
libraries must already be available. CI is Linux-only and has read-only access.
Clipboard, tray, shortcut and focus testing requires a desktop and explicit
interactive-test scope.

These conventions follow Codex's documented
[repository skills](https://developers.openai.com/codex/skills) and
[custom agent roles](https://developers.openai.com/codex/subagents).
