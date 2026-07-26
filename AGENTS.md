# Agent Rules for v8-runner

## Branches for New GitHub Issues

Every new task that comes from a GitHub issue must start in a separate new branch created from the current `master`.

1. Before planning or implementing an issue, check the current branch and worktree status.
2. Update `master` and create a new feature/fix branch from `master`, unless the user explicitly names a different base branch.
3. Do not start work for a new issue in an existing feature branch, even if that branch is clean.

## Mandatory Review Requirements

Any review of Rust code changes is incomplete without explicitly applying the `/rust-expert-best-practices-code-review` skill.

1. The **review subagent** must independently apply the checklist and rules from `/rust-expert-best-practices-code-review` when reviewing Rust changes, even if a separate Rust expert subagent is also run in parallel.
2. The **Rust expert subagent** remains a required independent review pass before commit and is not replaced by a normal reviewer pass.
3. If a task includes Rust code review, refactoring, error handling, type safety, API design, or performance-sensitive changes, `/rust-expert-best-practices-code-review` is mandatory.
4. Missing a separate pass with this skill is a review policy violation; findings from it must not be ignored silently. Each finding must be fixed or explicitly recorded as an accepted waiver with a short rationale.

## Task Classification

Use these categories to decide which review gates apply.

| Task type | Definition | Required sidecar pass when available |
| --- | --- | --- |
| Simple cosmetic edit | One file; spelling, formatting, wording, or comment-only cleanup; no semantic change to rules, specs, public contracts, command behavior, examples, fixtures, or tests. | Self-review is enough. |
| Non-cosmetic docs or rules | Documentation that changes agent obligations, user workflows, CLI/MCP/config contracts, `SKILL/SKILL.md`, `AGENTS.md`, `spec/`, ADRs, architecture docs, examples, or acceptance criteria. | Reviewer subagent; skeptic subagent when the edit changes rules, specs, ADRs, architecture decisions, or public contracts. |
| Rust implementation | Any `.rs` change, generated Rust, or test/fixture change that affects behavior. | Tester subagent for meaningful behavior risk; reviewer subagent; Rust expert review when Rust review/refactoring/API/error/performance is involved. |
| Public contract or architecture change | Changes to CLI/MCP surface, `v8project.yaml`, output/error contracts, `workPath`, source-set behavior, platform adapters, parsers, change detection, or documented invariants. | Explorer, skeptic before implementation, tester, reviewer, and reconciliation against relevant `spec/`/ADR/checklist artifacts. |

If a task fits multiple rows, apply the strictest row. If subagent tooling is unavailable, the main agent must perform the check locally and explicitly record the fallback.

## Mandatory During Work

For non-trivial tasks as classified above, use subagents during research and verification, not only before commit, when subagent tooling is available in the current environment.

1. **Explorer subagent** - for repo-wide file search, reading and gathering evidence, and finding affected modules, interfaces, and contracts.
2. **Tests/Tester subagent** - for running tests, smoke checks, and other verification (`cargo test`, targeted suites, integration checks).
3. **Reviewer subagent** - for independent review of risks, architectural consequences, and completeness when a task touches multiple modules, public contracts, or complex refactoring.
4. **Skeptic subagent** - for adversarial review of non-trivial plans, ADR/spec changes, public-contract decisions, broad refactoring proposals, and workflow/rule changes before implementation begins.
5. If subtasks are independent, run them in parallel. The main agent must integrate the results instead of doing all search and verification only locally.

The purpose of this rule is to keep implementation, search, review, and test evidence in separate independent passes while speeding up work in a large repository.

## Review Gate

Apply this gate at the stated phase for non-trivial changes, especially when they touch CLI contracts, `v8project.yaml` configuration, use-case orchestration, the domain model, platform/process adapters, MCP server/tools, parsers, change detection, output/reporting, fixtures, or public workflows documented in `SKILL/SKILL.md`.

1. Keep deterministic operations in the main session: project rule and specification updates, final verification, reconciliation of findings, staging, commits, and accepted-waiver records.
2. Before implementation begins, run `skeptic-review` for non-trivial plans, ADR/spec changes, architecture decisions, public-contract changes, broad refactoring, or workflow/rule changes. Critical or high skeptic findings block implementation until fixed or accepted by skeptic re-check; they cannot be waived by the agent alone.
3. For non-trivial, cross-module, or output-contract changes, run a fresh reviewer subagent on the current repository state before marking the task complete or committing. Findings must be fixed, re-reviewed, or explicitly recorded as non-actionable for the current task.
4. Accepted waivers or accepted risks require explicit user/maintainer approval or an existing ADR/spec decision. Record them in the final response or task notes; if they affect a public contract or architecture invariant, record them in the relevant `spec/` or ADR artifact before commit.
5. Before completing a non-trivial implementation that changes interfaces, adapters, public contracts, shared behavior, or multiple modules, review the actual diff with `mattpocock-skills:codebase-design`: verify that added modules, interfaces, seams, and adapters remain deep, match the approved plan or decision, and do not introduce shallow pass-through layers.
6. Treat these as blocking codebase-design findings when they duplicate an existing owner or public contract shape in the touched area: one-field wrappers, mirrored DTOs or enums, compatibility adapters without real external translation, duplicate registries or mappings, repeated readers/parsers/normalizers/loaders/scanners, alternate barrel/re-export surfaces with duplicate ownership, and multi-hop conversion chains. Exceptions are allowed only for a concrete external contract, a distinct invariant, an intentional crate-facade re-export, or a recorded project decision.
7. For cleanup work that removes duplication, enforces an invariant, or closes a documented regression in an area governed by a cleanup note, ADR, architecture invariant, `spec/architecture/change-checklist.md`, issue/PR note, or guardrail test, require a `Reintroduction guard`. The guard must name the root cause, the single owner, and a way to detect the same problem reappearing under a different name. The guard may be a test, architecture guardrail, lint/check, fixture/snapshot, or explicit review checklist entry.
8. Before completion or commit, reconcile only against artifacts explicitly cited by the task or directly relevant to touched files/public contracts: approved plan, ADR, `spec/architecture/invariants.md`, `spec/architecture/change-checklist.md`, other relevant `spec/` architecture documents, issue/PR notes, skeptic review, or explicit task notes. Compare the actual diff against the stated contracts, invariants, ownership rules, data-flow paths, conversions, mappings, registries, and public re-exports; verify that promised deletions happened. An unlisted addition or retained duplicate requires updating the relevant plan or decision and repeating review before commit.
9. Actionable non-trivial findings must be fixed, re-reviewed, waived by the rule above, or marked out of scope only when they are unrelated to the current task. When worker subagents are available, use at most one focused worker/fix pass per related finding group before re-review.

## Repo-Local Skill

`SKILL/SKILL.md` is the skill for using `v8-runner` in other 1C projects. When implementing tasks, update it if commands, workflows, configuration contracts, constraints, or diagnostic practices that matter for external use change.

Changes to `SKILL/SKILL.md` must be short, dense, and practical: add only applicable instructions, without restating internal implementation details or bloating the reference.

## Commit message format

```
feat(scope): short description

- bullet points of what was done
```
