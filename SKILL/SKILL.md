---
name: v8-runner
description: "Use when Codex needs to operate v8-runner on local 1C projects from the CLI: configure v8project.yaml, initialize infobases or EDT workspaces, build Designer or EDT sources, run syntax checks and tests, dump infobase changes, convert source formats, load or export artifacts, launch 1C clients, or choose safe 1C automation command sequences."
---

# v8-runner

Use this skill to operate `v8-runner` as the automation layer for local 1C development projects.

Keep this file as the decision entrypoint. Load only the reference file that matches the task:

- `references/command-selection.md` for choosing the right command sequence.
- `references/config-and-backends.md` for `v8project.yaml`, source sets, formats, per-operation providers, and backend limits.
- `references/project-workflows.md` for common build, syntax, dump, launch, and source sync workflows across Designer and EDT projects.
- `references/file-and-artifact-workflows.md` for dump, convert, load, make/artifacts, and staged publication.
- `references/testing.md` for YaXUnit, Vanessa Automation, syntax checks, and artifacts.
- `references/troubleshooting.md` for setup failures, stale state, and environment diagnostics.

## Command Form

Use the available `v8-runner` binary directly. If it is not on `PATH`, ask for the binary path or use a project-provided wrapper script.

`v8project.yaml` is the default project config name. A sibling `v8project.local.yaml` is loaded automatically for machine-local paths, credentials, tools, tests, and MCP settings. Do not pass `--config v8project.yaml` unless the user explicitly wants a non-default command shape or the active config path differs from the default; never pass `v8project.local.yaml` as `--config`.

Generated `v8project.yaml` files include a `yaml-language-server` modeline that points to the published `master` JSON Schema artifact. `config init` and `bootstrap` also create sibling `v8project.local.yaml` with the local overlay schema modeline and add it to `.gitignore` when needed.

Use JSON output only when another tool, script, or final answer needs structured results:

```bash
v8-runner --json-message build
```

Use text output for direct human diagnostics.

Use `v8-runner version` or `v8-runner --version` to check the installed application version; it does not require `v8project.yaml`.

Useful global flags:

- `--version` to print the application version and exit.
- `--config <CONFIG>` when the active config is not `./v8project.yaml`.
- `--json-message` for machine-readable CLI envelopes.
- `--workdir <WORKDIR>` to override `workPath`; it wins over `v8project.local.yaml`.
- `--clean-before-execution` to clear logs before execution.
- `--log-level <error|warn|info|debug|trace>` for diagnostics.
- `--no-color` for plain text output.

## First Pass

1. Check whether `v8project.yaml` exists in the 1C project root.
2. If it is missing and source files already exist, run the narrowest `v8-runner config init ...` command that fits the project shape.
3. If it is missing and the only goal is to export CF/CFE/DT from an existing infobase, create a
   minimal `v8project.yaml` with `workPath`, `format`, `infobase`, platform discovery settings and
   `source-set: []`; do not bootstrap project sources that the user did not request.
4. If it is missing and the current source of truth is an existing infobase that must become
   project sources, run `v8-runner bootstrap --connection <CONNECTION> --platform-version <VERSION>`.
5. Inspect generated `v8project.yaml` and keep machine-local overrides in generated `v8project.local.yaml`.
6. Run `v8-runner init` only when the file infobase or EDT workspace needs to be created.
7. Run the narrowest validation command that answers the user's goal.

Minimal infobase-only shape:

```yaml
workPath: build/v8-runner
format: DESIGNER
infobase:
  connection: "File=/absolute/path/to/ib"
source-set: []
```

Useful bootstrap commands:

```bash
v8-runner config init
v8-runner config init --connection "File=build/ib"
v8-runner config init --format edt
v8-runner bootstrap --connection "File=/path/to/ib" --platform-version 8.3.27
v8-runner tools download yaxunit --sources
v8-runner tools download vanessa
v8-runner tools download client-mcp --sources
v8-runner init
```

## Default Use-Case Routing

- Source files changed and infobase may be stale: run `v8-runner build`.
- Only one source-set changed: use commands that accept `--source-set <NAME>` instead of rebuilding or materializing everything.
- Branch switch, rebase, large object moves, stale source-backed tool extension state, or suspicious incremental state: run `v8-runner build --full-rebuild`.
- Syntax check: inspect `format`, then choose `syntax designer-modules`, `syntax designer-config`, or `syntax edt`. `syntax` has one executor (Designer) and takes no `providers` key.
- Behavior validation: run the relevant `v8-runner test ...` command; tests build first unless the
  caller explicitly requests `--no-build` for an already prepared infobase.
- Missing local YAxUnit, Vanessa Automation, or onec-client-mcp-devkit setup: run
  `v8-runner tools download yaxunit --sources`, `v8-runner tools download vanessa`, and
  `v8-runner tools download client-mcp --sources` for source-backed setup. Omit
  `--sources` on `yaxunit` or `client-mcp` to download `.cfe` artifacts; loading a
  `.cfe` needs the Designer executor.
- Vanessa Automation debugging or scenario authoring: use `v8-runner launch mcp va --wait-ready ...` to start the client MCP server with VA loaded and verify the VA MCP tools before driving `.feature` workflows.
- Extension properties need synchronization: use `v8-runner extensions` or `extensions --name <SOURCE_SET>`.
- Infobase changes need to become Git-visible files: check `git status`, then run the relevant `v8-runner dump ...` command.
- Need a CF/CFE package of the state currently stored in the infobase: use
  `v8-runner infobase configuration export --state <working|database> --output <file.cf>`;
  add `--extension <name>` and use `.cfe` for an extension. This is not `make`, which builds
  artifacts from project sources.
- Need a complete portable DT image including data: use `v8-runner infobase dump --output <file.dt>`.
  A DT is not a backup. The executor comes from the matrix (`providers.infobase.dump`),
  experimental IBCMD DT is skipped unless named explicitly, and a ready Designer is
  selected before spawn when available.
- For infobase export failures, distinguish `capability_unavailable` (no implemented adapter)
  from `environment_unavailable` (adapter exists, but binary/version/connection is not ready).
  Never retry another provider after the selected provider has been spawned.
- Before an orchestrator applies either infobase export, append `--dry-run` to obtain the exact
  provider selection and output plan without creating `workPath`, locks, output paths, or a
  platform process. Treat `mode=preview` and `provider_dispatched=false` as the non-execution proof.
- Need to load a complete infobase back from a DT image: use
  `v8-runner infobase restore --input <file.dt>` with exactly one target mode — `--replace` to
  discard the data of an existing infobase, `--create` to create an absent one. A mode that does
  not match the observed target is refused before the platform starts; neither provider asks, and
  there is no staging step that could undo a load. Append `--dry-run` first to see the selected
  provider and the planned input without touching the infobase.
- Before any command that starts the platform or touches the infobase, append `--dry-run` to see
  what it would do: `init`, `build`, `load`, `apply`, `reset`, `dump`, `convert`, `artifacts`, `launch`,
  `infobase restore` and both `infobase` exports accept it. A preview locates the platform first,
  so a missing one is refused before the plan is approved, and it takes no locks and creates
  no target artifacts (legacy commands still log the preview; `apply`/`reset` write no files),
  and it neither takes nor waits for the workspace lock, so a preview works while
  another command holds it. Proof that nothing ran: `provider_dispatched: false` for the
  launch-shaped verbs, `mode: preview` for the `infobase` ones — each form carries its own
  closed signal. Two limits are named
  rather than guessed: `load` reports `compatibility_state: not_probed` because the probe is
  itself a Designer run, and `init` against a server infobase cannot tell "created" from
  "already existed" without creating it.
- Source files need conversion between Designer and EDT: use `v8-runner convert`; this is CLI-only and does not use the infobase.
- Existing `.cf` or `.cfe` artifacts need to be applied to an infobase: use `v8-runner load ...`.
  To load only the working configuration, add `--no-apply`; the receipt has `applied: true`
  and `update_db_cfg_ran: false`. Run `apply` separately to update the database configuration.
  A failed update can still have `applied: true`; inspect the receipt before retrying.
- Apply the working configuration to the database with `v8-runner apply`; discard
  unapplied configuration edits with `v8-runner reset --force`. Use `--extension NAME`
  on either command to address an extension. Reset does not delete extensions or restore
  infobase data; force acknowledges that generation protection is unavailable. Preview
  with `--dry-run` first (reset still needs force). Both commands are CLI-only Designer operations.
- Release artifacts need to be exported or external artifacts published: use `v8-runner make ...` or the `artifacts` alias.
- Need to know which extensions are installed in an infobase, or to change that composition:
  use `v8-runner extensions list|info|create|delete|activate`. These subcommands address the
  infobase, not the workspace — bare `v8-runner extensions` still means "update the security
  properties of the configured extension source-sets". For `ibcmd`, a successful read
  reports `name_prefix` from the applied DB configuration; for the standalone agent it is
  `null` until that provider can attest the applied prefix. Never fill it from source files
  or the working configuration after `upload` without `apply`.
  Every subcommand of this family, reads included, accepts `--dry-run`: reading the composition
  starts the platform, authenticates and leaves a journal trace, so it is an action. The preview names the
  target infobase, the account and the utility, and never echoes the connection string.
- Need a 1C UI session: use `v8-runner launch designer`, `launch thin`, `launch thick`, or `launch ordinary`.
- Need the thin client against a published base: `launch thin --via web` opens `infobase.web.url` as a ws connection. A standalone-server target takes that path by default — it has no other address — while `launch web` still opens the same address in a browser. `--via` is accepted only where the client is thin.
- Need to know which binary and arguments a launch would use without starting a client: append
  `--dry-run` to `launch designer|thin|thick|ordinary`. It returns `provider_dispatched=false`,
  `pid=null`, and a `plan` with the selected `program` and the composed `args`; credential values
  inside `plan.args` are replaced by `***`, so the plan is readable but not reusable as a manual
  command line. It cannot be combined with `--wait-for-exit` or `--wait-ready`.
- Need to read a failed launch: the command in the error text and in `logs/mcp/actions.log` is
  masked the same way, and there the user name is hidden too (`/N ***`, `Usr=***`) because those
  lines outlive the run. Server, base and paths stay readable; run `--dry-run` to see the account.
- Need an observable local external EPF runtime gate: use `launch thin --execute <file.epf> --output <out> --stderr-output <stderr> --wait-for-exit --wait-timeout-ms <ms>`. This opt-in mode is limited to explicit `.epf` files, reports PID/exit-or-timeout/artifacts, treats timeout as a CLI failure after terminating the client group, and rejects raw or configured `/C`, `/Execute`, and `/Out` aliases; callers must inspect the reported exit code because non-zero EPF exit is observational rather than a CLI failure; plain launch remains asynchronous.
- Need onec-client-mcp-devkit launched inside 1C without VA authoring: use `v8-runner launch mcp --wait-ready ...` when the caller needs a ready MCP endpoint; tune readiness with `tools.client_mcp.wait_ready_timeout_ms` when the project needs a shorter or longer wait, raise `execution_timeout` too when extending beyond the global command budget, and use bare `launch mcp` only for fire-and-forget startup.

## Guardrails

- Do not delete or recreate an infobase, workspace, temp directory, or generated state unless the user explicitly asks or the command itself is the documented recovery path.
- Never pass `infobase restore --replace` to recover from a failed command: it discards the data of the target infobase, and `target_state: uncertain` after a failed restore means an unknown amount of data was already replaced. Dump first.
- Do not invent raw `1cv8`, `ibcmd`, or `1cedtcli` flags; prefer the `v8-runner` command surface.
- Check `git status` before `dump` when the result may overwrite or mix with existing source changes.
- Preserve failed test artifacts under `workPath/temp/<runner-id>/runs/<run-id>/` for diagnosis instead of cleaning them immediately.
- Report missing local 1C utilities as environment/setup issues, not as project source failures.
- Keep final answers concrete: command run, result, relevant artifact path, and any follow-up command.

## Output Discipline

When reporting results, distinguish:

- project source failures;
- v8-runner command/config failures;
- local 1C platform, EDT, IBCMD, or tool discovery failures;
- test failures and their artifact paths.
