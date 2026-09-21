# Archive Guide

This directory stores historical internal documents that are kept for context, not as active
project truth.

Use these files only when you need delivery history, migration context, or old design intent.
Do not sync public docs, ADR checklists, or implementation work against archive files.

Current active entry points:

- GitHub issues репозитория (сводный план: #233)
- `../ADR_DERIVED_BACKLOG.md`
- `../acceptance/real-environment-validation.md`
- `../../docs/README.md`

Historical records intentionally kept here:

- `IMPLEMENTATION_TODO_2026-04-30.md`: closed task ledger moved out of the active TODO.
- `ADR_DERIVED_BACKLOG_2026-04-30.md`: full ADR reconciliation snapshot before cleanup.
- `MCP_IMPLEMENTATION_PLAN_2026-03-21.md`: unique closed MCP rollout history.
- `KEY_COMPONENTS_legacy.md`: legacy component map kept only as migration-era maintainer context.

Removed during the 2026-04-30 truth-layer cleanup:

- dated TODO snapshots `IMPLEMENTATION_TODO_2026-04-21.md` and `IMPLEMENTATION_TODO_2026-04-23.md`
  because `IMPLEMENTATION_TODO_2026-04-30.md` already preserves the closed ledger that mattered;
- `FUNCTIONAL_CAPABILITIES_legacy.md` and `IMPLEMENTATION_BACKLOG_legacy.md` because their useful
  context is superseded by the active public docs, `ARCHITECTURE.md`, ADRs, and the retained MCP
  rollout record.
