---
name: v8-runner
description: "Use when Codex needs to operate v8-runner on local 1C projects from the CLI: configure v8project.yaml, initialize infobases or EDT workspaces, build Designer or EDT sources, run syntax checks and tests, dump infobase changes, convert source formats, load or export artifacts, launch 1C clients, or choose safe 1C automation command sequences."
---

# v8-runner

Use this skill to operate `v8-runner` as the automation layer for local 1C development projects.

Keep this file as the decision entrypoint. Load only the reference file that matches the task:

- `references/command-selection.md` for choosing the right command sequence.
- `references/config-and-backends.md` for `v8project.yaml`, source sets, formats, per-operation providers, and backend limits.
- `references/project-workflows.md` for common push, check, pull, launch, and source sync workflows across Designer and EDT projects.
- `references/file-and-artifact-workflows.md` for pull, convert, upload, make/artifacts, and staged publication.
- `references/testing.md` for YaXUnit, Vanessa Automation, syntax checks, and artifacts.
- `references/troubleshooting.md` for setup failures, stale state, and environment diagnostics.

## Command Form

Use the available `v8-runner` binary directly. If it is not on `PATH`, ask for the binary path or use a project-provided wrapper script.

`v8project.yaml` is the default project config name. A sibling `v8project.local.yaml` declares the project's infobases (`infobases` map, `origin` by default) and holds machine-local paths, credentials, tools, tests, and MCP settings. Do not pass `--config v8project.yaml` unless the user explicitly wants a non-default command shape or the active config path differs from the default; never pass `v8project.local.yaml` as `--config`. Relative paths in both files resolve from the directory of `v8project.yaml`, also when `--config` points into a nested directory.

Generated `v8project.yaml` files include a `yaml-language-server` modeline that points to the published `master` JSON Schema artifact. `init` and `clone` also create sibling `v8project.local.yaml` with the local overlay schema modeline and add it to `.gitignore` when needed, together with `ConfigDumpInfo.xml` and `.dump-*.lock*`; the file is the project directory's `.gitignore` (the `init` working directory or `clone --project-dir`, never a worktree root above it; unanchored patterns cover every source set and a nested `--output`), and only a `.gitignore` inside the worktree counts as coverage, not `.git/info/exclude` or a global excludes file. `pull` and `push` refuse with exit code 2 before starting the platform when `ConfigDumpInfo.xml` is tracked by git (the file belongs to one infobase); follow the printed `git rm --cached` recipe.

Use JSON output only when another tool, script, or final answer needs structured results:

```bash
v8-runner --json-message push
```

Use text output for direct human diagnostics.

Use `v8-runner version` or `v8-runner --version` to check the installed application version; it does not require `v8project.yaml`.

Useful global flags:

- `--version` to print the application version and exit.
- `--config <CONFIG>` when the active config is not `./v8project.yaml`.
- `--json-message` for machine-readable CLI envelopes.
- `--workdir <WORKDIR>` to override `workPath`; it wins over `v8project.local.yaml`.
- `--infobase <NAME|CONNECTION>` to work with another declared infobase or an ad hoc connection string; defaults to `origin`.
- `--clean-before-execution` to clear logs before execution.
- `--log-level <error|warn|info|debug|trace>` for diagnostics.
- `--no-color` for plain text output.

## First Pass

1. Check whether `v8project.yaml` exists in the 1C project root.
2. If it is missing and source files already exist, run the narrowest `v8-runner init ...` command that fits the project shape.
3. If it is missing and the only goal is to export CF/CFE/DT from an existing infobase, create a
   minimal `v8project.yaml` with `workPath`, `format`, platform discovery settings and
   `source-set: []`, plus a sibling `v8project.local.yaml` with `infobases.origin.connection`; do not bootstrap project sources that the user did not request.
4. If it is missing and the current source of truth is an existing infobase that must become
   project sources, run `v8-runner clone --from <CONNECTION> --platform-version <VERSION>`.
   It writes only into an empty directory (nothing but `.git`); a non-empty one is refused with
   exit code 2 before anything is written, unless `--force`.
   Add `--dry-run` first: it names the four paths it would write and the dump utility it found,
   and creates nothing — not even the project directory.
5. If it exists but the worktree has its own infobase to point at (a new worktree, a copied `v8project.local.yaml`), run `v8-runner init --infobase "File=build/ib"`: it leaves `v8project.yaml` untouched, writes only the local layer, and keeps the previous `origin` with its credentials as `upstream` (refused if `upstream` exists).
6. Inspect generated `v8project.yaml` and keep machine-local overrides in generated `v8project.local.yaml`.
7. Run `v8-runner infobase create` only when the file infobase or EDT workspace needs to be created.
8. Run the narrowest validation command that answers the user's goal.

Minimal infobase-only shape (two files):

```yaml
# v8project.yaml
workPath: build/v8-runner
format: DESIGNER
source-set: []
```

```yaml
# v8project.local.yaml
infobases:
  origin:
    connection: "File=/absolute/path/to/ib"
```

`infobase:` in either file is a one-cycle synonym for `infobases.origin` and warns.

Useful setup commands:

```bash
v8-runner init
v8-runner init --infobase "File=build/ib"
v8-runner init --format edt
v8-runner clone --from "File=/path/to/ib" --platform-version 8.3.27
v8-runner tools download yaxunit --sources
v8-runner tools download vanessa
v8-runner tools download client-mcp --sources
v8-runner infobase create
```

## Default Use-Case Routing

- Source files changed and infobase may be stale: run `v8-runner push`.
- Only one source-set changed: name it positionally (`push <SET>`, `pull <SET>`, `make <SET>`, `download <SET>`, `convert <SET>`) instead of rebuilding or materializing everything. A positional value is always a source set, never a base: name the base with `--infobase`.
- Package vs whole base: `.cf`/`.cfe` is `download`/`upload <FILE>`, `.dt` is `infobase dump`/`infobase restore`; `infobase dump --output *.cf|*.cfe` is refused before the platform starts and names `download`; `upload *.dt` names `infobase restore`. `download --state db` takes the database configuration through Designer or `ibcmd` (chain order); `providers.download: agent` and a standalone server (agent only) refuse it with `capability_unavailable` before the platform starts.
- After successful full `pull` in `DESIGNER` format, the next unchanged `push` skips loading for the same named base/source set. If the response says sources were published without updating hash memory, repeat full `pull`; do not repair a failed pull by pushing old sources.
- Hash memory is separate per named base; ad hoc connection strings do not reuse it. Foreign memory is named in the refusal: full `pull` if the base is right, `push --full` if the sources are right. Memory from older runner versions is not migrated: first `pull --force` before an ordinary `push`, or the push loads the whole tree.
- Full `pull` refuses a target containing `workPath`, including symlink aliases. EDT export cache stays shared; per-base agent generation/version-file memory remains pending in #214.
- Branch switch, rebase, large object moves, stale source-backed tool extension state, or suspicious incremental state: run `v8-runner push --full`.
- Configuration check: run `v8-runner check`. The project `format` picks the branch — `/CheckConfig` for DESIGNER, EDT validation for EDT — and a key the branch does not execute is refused. With no mode key the default profile runs; name modes to narrow it. One executor (Designer), no `providers` key. A project of external data processors and reports only is refused with `error.code: subject`. `--dry-run` stops after the utility is located and before the platform runs: no platform log directory is created, and the answer names `status: planned`, `provider_dispatched: false` and `exit_code: -1`.
- EDT check with `interactive-mode: true` reads the shared session's verdict exactly as MCP `check_syntax_edt` does: any stderr or stdout without log issues is `tool_failed`, `exit_code` is `101` for issues and `-1` for a failure, and `tools.edt_cli.command_timeout_ms` bounds each project.
- Behavior validation: run the relevant `v8-runner test ...` command; tests run `push` first unless the
  caller explicitly requests `--no-push` for an already prepared infobase.
- Missing local YAxUnit, Vanessa Automation, or onec-client-mcp-devkit setup: run
  `v8-runner tools download yaxunit --sources`, `v8-runner tools download vanessa`, and
  `v8-runner tools download client-mcp --sources` for source-backed setup. Omit
  `--sources` on `yaxunit` or `client-mcp` to download `.cfe` artifacts; loading a
  `.cfe` needs the Designer executor. Tools come from the GitHub `releases/latest`;
  `tools download vanessa --prerelease` takes the highest version including pre-releases.
- Vanessa Automation debugging or scenario authoring: use `v8-runner launch mcp va --wait-ready ...` to start the client MCP server with VA loaded and verify the VA MCP tools before driving `.feature` workflows.
- Extension security properties: use `extensions --name <SOURCE_SET>` or
  `extensions --installed-name <PLATFORM_NAME>` for a separately loaded CFE such as YAXUNIT.
  Repeat/combine selectors for explicit targets; neither selector means all configured extensions.
  Append `--dry-run` to preview without platform calls. Apply disables safe mode and unsafe action protection.
- Infobase changes need to become Git-visible files: check `git status`, then run the relevant `v8-runner pull ...` command.
- Need a CF/CFE package of the state currently stored in the infobase: use
  `v8-runner download [--state db] --output <file.cf>` (without `--state` the working configuration, `db` the database one);
  add `--extension <name>` and use `.cfe` for an extension. This is not `make`, which builds
  artifacts from project sources.
- Need a complete portable DT image including data: use `v8-runner infobase dump --output <file.dt>`.
  A DT is not a backup. The executor comes from the matrix (`providers.infobase.dump`),
  experimental IBCMD DT is skipped unless named explicitly, and a ready Designer is
  selected before spawn when available.
- `error.kind` and `error.code` are closed enumerations. Within `capability`, the code says why:
  `capability_unavailable`, `target` (not for this target), `soon` (not yet). A refusal that has a
  way out names it in `error.next` — `{command, source_set?, keys?}` — so an orchestrator reads the
  step instead of parsing the message.
- A busy `workPath` (another run holds its lock) answers at once with `error.kind: workspace`,
  `error.code: workspace_busy`, step `workspace lock` and exit 3 for every CLI command; MCP
  answers `runtime_failure`. Wait for the other run and retry.
- A file infobase is held for the whole command by a lock next to its directory, taken after the
  `workPath` lock: a second command on the same base — even from another working copy — answers
  at once `error.code: infobase_busy` (kind `workspace`, step `infobase lock`, exit 3) and names
  the holding command and its `workPath`; MCP answers `runtime_failure`. Retry when it finishes.
  If the lock cannot be taken for another reason (the directory next to the base is read-only),
  a writing command refuses with the directory and the reason, and a reading one (`download`,
  `infobase dump`, `make`, `extensions list`) goes on with an `infobase lock …` warning.
- When an operator's interrupt (Ctrl+C, SIGTERM) ends a command, the CLI envelope answers
  `error.kind: interruption`, `error.code: cancelled` and exit 4 for every command; MCP folds it
  into `platform_failure`. A pending interrupt alone decides nothing: an unrelated failure keeps
  its own code, and a critical phase such as a database write runs to its end — a success stays
  a success and names the interrupt with `deferred: true`. In `upload`, `infobase restore`, `push`,
  `extensions` and `infobase create` a later failure or a stop at the next safe point names it too
  (forms without `execution` say it in the step message). In forms with `execution`, the
  interruption record's `phase` says where it stopped: `command_boundary` — a safe point, no work
  of the command was cut short; `provider_command`, `run`, `apply`, `update_db_cfg`,
  `publication` — the executor's work was cut short or, with `deferred: true`, waited for.
  When the command was stopped (`status: cancelled`), `test`, `upload`, `make`, `download`,
  `infobase dump` and `infobase restore` also put `{code: "cancelled"}` in `execution.errors[]`
  next to that record; a `deferred: true` record carries no such error.
- A `providers.*` key naming an executor outside the matrix is refused at config load with
  `invalid_argument` (exit 2, message lists the implemented executors); fix the key, do not
  retry. `download`, `infobase configuration export`, `infobase dump` and `infobase restore`
  each check only the key of their own operation (`download`, `infobase.dump` or
  `infobase.restore`), so a key of another operation does not block them; `test --no-push` and `launch` check no key; every
  other command that loads the project checks all keys.
- `provider.endpoint` (`mode`: `managed`/`attached`/`gate`, `address`: `host:port`, never credentials) appears only when the command opened an agent session; previews and platform-process runs omit it.
- For infobase export failures, distinguish `capability_unavailable` (no implemented adapter)
  from `environment_unavailable` (adapter exists, but binary/version/connection is not ready).
  Never retry another provider after the selected provider has been spawned.
- Before an orchestrator applies `download` or `infobase dump`, append `--dry-run` to obtain the exact
  provider selection and output plan without creating `workPath`, locks, output paths, or a
  platform process. Treat `mode=preview` and `provider_dispatched=false` as the non-execution proof.
- Need to load a complete infobase back from a DT image: use
  `v8-runner infobase restore --input <file.dt>` with exactly one target mode — `--replace` to
  discard the data of an existing infobase, `--create` to create an absent one. A mode that does
  not match the observed target is refused before the platform starts; neither provider asks, and
  there is no staging step that could undo a load. Append `--dry-run` first to see the selected
  provider and the planned input without touching the infobase.
- `pull` modes come from dictionary keys: no key — incremental dump over the directory;
  `--object <TYPE:NAME>` — partial; `--force` — full dump that replaces the directory. The hidden
  `--mode incremental|partial` means no key; `--mode full` is refused and names `pull [SET] --force` for the same set.
  `--force` next to `--object` or `--mode` is refused before the platform; pick one form.
- In an EDT-format project every `pull` (no key, `--object`) replaces the whole project directory.
  Without `--force`, uncommitted work there makes it refuse: commit or stash it and repeat, or
  run the exact `v8-runner --config … pull <SET> --force` the refusal names (full dump, uncommitted
  work is lost; never add `--force` to a call with `--object` or `--mode` — that is refused).
  There is no partial EDT pull with consent.
- `convert`, any EDT-format `pull` without `--force` and MCP `dump_config` with `FULL` (any mode
  in an EDT project) replace the target source directory as a whole, so they first ask git what
  inside it exists nowhere else — untracked files, ignored files, a worktree edit on top of the
  index, unresolved merge markers. Finding any, the command refuses before touching anything with
  exit 2 and names them. Commit or stash them and repeat, or run the command the refusal names
  as written: for `pull`/MCP `dump_config` an exact `v8-runner --config <abs> [--infobase …]
  [--workdir …] pull <SET> --force` (MCP over HTTP: on the server's machine; a connection-string
  `--infobase` is not repeated — add the same `--infobase` value yourself); for `convert` the
  same command with `--force` added — never drop the set or `--output`. The flag destroys them and keeps no
  copy, so check `git status` first. `clone` has no such consent: commit or stash.
  Staged content is not a loss: it is
  recoverable from the index. Where git cannot answer — no git, outside a worktree, a git error, a
  directory git could not read — the command proceeds exactly as it did before this check existed,
  and the guard claims no protection there.
- `--dry-run` is a global key: it means the same before and after the command. Commands with no
  preview — `version`, `init`, `tools download`, `test`, `mcp serve` — refuse it
  with a named reason instead of running.
- Before any command that starts the platform or touches the infobase, append `--dry-run` to see
  what it would do: `clone`, `infobase create`, `push`, `upload`, `pull`, `convert`, `artifacts`,
  `launch`, `check`, `infobase restore`, `download` and `infobase dump` accept it. A preview locates the platform first,
  so a missing one is refused before the plan is approved, and it takes no locks and creates
  nothing — not the target, not `workPath`, not the action log — so a preview also runs under a
  read-only sandbox. The record of the call is the envelope on stdout, not a log file. It
  neither takes nor waits for the workspace or infobase lock, so a preview works while
  another command holds them. `provider_dispatched: false` means no executor got the
  command's work; export-shaped verbs also answer `mode: preview`. The flag is not a preview
  marker — refusals before any work and runs with nothing to do answer `false` too — so know
  the preview from your own `--dry-run`. `true` means an executor got the work: a failure
  with `true` is not "nothing ran", so check the target before a retry. A failure after the
  executor got the work answers the command's own form; the shared refusal form, without the
  flag, means no executor got work. Two limits are named
  rather than guessed: `upload` reports `compatibility_state: not_probed` because the probe is
  itself a Designer run, and `infobase create` against a server infobase cannot tell "created" from
  "already existed" without creating it.
- Source files need conversion between Designer and EDT: use `v8-runner convert`; this is CLI-only and does not use the infobase.
- Existing `.cf` or `.cfe` artifacts need to be applied to an infobase: use `v8-runner upload ...`.
- Release artifacts need to be exported or external artifacts published: use `v8-runner make ...` or the `artifacts` alias.
- Need to know which extensions are installed in an infobase, or to change that composition:
  use `v8-runner extensions list|info|create|delete|activate`. These subcommands address the
  infobase, not the workspace — bare `v8-runner extensions` still means "update the security
  properties of the configured extension source-sets". For `ibcmd`, a successful read
  reports `name_prefix` from the applied DB configuration; for the standalone agent it is
  `null` until that provider can attest the applied prefix. Never fill it from source files
  or the working configuration after `upload` without `apply`. A read that fails after the
  platform got the request answers `ok: false` with an empty `extensions`: the composition is
  unknown, not empty — check `ok` first.
  Every subcommand of this family, reads included, accepts `--dry-run`: reading the composition
  starts the platform, authenticates and leaves a journal trace, so it is an action. The preview names the
  target infobase, the account and the utility, and never echoes the connection string.
- Need a 1C UI session: use `v8-runner launch designer`, `launch thin`, `launch thick`, or `launch ordinary`.
  Launch uses the configured infobase and client settings; no source-set is required.
- Need the thin client against a published base: `launch thin --via web` opens `infobase.web.url` as a ws connection. A standalone-server target takes that path by default — its direct gate address, when declared, is not used by the runner yet — while `launch web` still opens the same address in a browser. `--via` is accepted only where the client is thin.
- Need to know which binary and arguments a launch would use without starting a client: append
  `--dry-run` to `launch designer|thin|thick|ordinary`. It returns `provider_dispatched=false`,
  `pid=null`, and a `plan` with the selected `program` and the composed `args`; credential values
  inside `plan.args` are replaced by `***`, so the plan is readable but not reusable as a manual
  command line. It cannot be combined with `--wait-for-exit` or `--wait-ready`.
- Need to read a failed launch: the command in the error text and in `logs/mcp/actions.log` is
  masked the same way, and there the user name is hidden too (`/N ***`, `Usr=***`) because those
  lines outlive the run. Server, base and paths stay readable; run `--dry-run` to see the account.
- Need an observable local external EPF runtime gate: use `launch thin --execute <file.epf> --output <out> --stderr-output <stderr> --wait-for-exit --wait-timeout-ms <ms>`. This opt-in mode is limited to explicit `.epf` files, reports PID/exit-or-timeout/artifacts, treats timeout as a CLI failure after terminating the client group, answers an interrupted wait with `exit_code: null` and `timed_out: false`, and rejects raw or configured `/C`, `/Execute`, and `/Out` aliases; callers must inspect the reported exit code because non-zero EPF exit is observational rather than a CLI failure; plain launch remains asynchronous.
- Need onec-client-mcp-devkit launched inside 1C without VA authoring: use `v8-runner launch mcp --wait-ready ...` when the caller needs a ready MCP endpoint; tune readiness with `tools.client_mcp.wait_ready_timeout_ms` when the project needs a shorter or longer wait, and use bare `launch mcp` only for fire-and-forget startup.

## Guardrails

- Do not delete or recreate an infobase, workspace, temp directory, or generated state unless the user explicitly asks or the command itself is the documented recovery path.
- Never pass `infobase restore --replace` to recover from a failed command: it discards the data of the target infobase, and `target_state: uncertain` after a failed restore means an unknown amount of data was already replaced. Dump first.
- Do not invent raw `1cv8`, `ibcmd`, or `1cedtcli` flags; prefer the `v8-runner` command surface.
- Check `git status` before `pull` when the result may overwrite or mix with existing source changes.
- Give each git worktree or clone its own file infobase (`origin` in its own `v8project.local.yaml`):
  the runner does not yet detect two working copies sharing one base; pushes from different
  branches silently mix in it, and a test run in one copy blocks apply in the other.
- Preserve failed test artifacts under `workPath/temp/<runner-id>/runs/<run-id>/` for diagnosis instead of cleaning them immediately.
- Report missing local 1C utilities as environment/setup issues, not as project source failures:
  a missing or wrong-version utility answers `environment_unavailable` (exit 2).
- Keep final answers concrete: command run, result, relevant artifact path, and any follow-up command.

## Output Discipline

When reporting results, distinguish:

- project source failures;
- v8-runner command/config failures;
- local 1C platform, EDT, IBCMD, or tool discovery failures;
- test failures and their artifact paths.
