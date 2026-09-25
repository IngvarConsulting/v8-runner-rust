# Config And Backends

Inspect `v8project.yaml` before diagnosing push, check, pull, test, and launch behavior.
If a sibling `v8project.local.yaml` exists, inspect it too because it overrides machine-local
settings before CLI overrides.

## Fields To Check First

- `workPath`: generated state, temp files, and workspace location.
- `format`: `DESIGNER` or `EDT`.
- `providers.<operation>`: optional per-operation executor override. Omit it to use the defaults below.
- `infobases.origin.connection` in `v8project.local.yaml`: the default infobase, often `File=build/ib`;
  a declared two-part `Srvr=…;Ref=…` with a single machine reaches the platform as
  `/S host[:port]\ref` with `/N`/`/P` taken from `user`/`password` — keep credentials out of the
  string itself; a string with extra parts (`Usr=`, `Locale=`, a comma-separated server list) is
  passed whole and keeps the old form;
  other declared names are picked with `--infobase <name>`. `infobase:` is a one-cycle synonym for
  `infobases.origin`.
- `source-set`: ordered configuration and extension sources.
- `tools.platform.path`, `version`, and `strict`: platform discovery hints. `path` is always an
  explicit-only boundary with no default-root or `PATH` fallback. Without `path`, `version` filters
  normal discovery. With `path`, `version` is ignored unless `strict: true`; strict path+version
  resolution rejects unknown or mismatched versions and pins sibling utilities to one canonical root.
- `tools.edt_cli.path`, `version`, and `interactive-mode`: EDT CLI discovery and execution mode.
- `tests.yaxunit` and `tests.va`: test runner configuration.
- `tools.client_mcp`, `tools.va`, and `tools.enterprise`: launch and client-side MCP integration hints.
- `tools.client_mcp.extension`: optional tool extension prepared by `push`; it is not a project `source-set`.
- `tools.client_mcp.wait_ready_timeout_ms`: optional readiness timeout for `launch mcp --wait-ready`; defaults to five minutes and is capped by nothing else — a command has no deadline.

## Choosing The Executor

There is no global backend switch. The executor is chosen per operation from a capability
matrix, and `providers.<operation>` names one explicitly.

- Executors are `designer`, `ibcmd`, and `agent`.
- The key is accepted only where the matrix gives this target more than one executor. On a file
  or cluster infobase that means `infobase.create`, `push`, `pull`, `extensions`,
  `download`, `infobase.dump`, `infobase.restore`, and `make`. Naming an
  executor for a single-executor operation is a config error, so `providers.upload` and
  `providers.syntax` are refused — both are Designer-only.
- Defaults: `infobase.create`, `push`, `pull`, `download` try Designer then `ibcmd`;
  `infobase.dump` and `infobase.restore` use Designer (experimental IBCMD DT only when named);
  `upload` and `syntax` are Designer-only; `extensions` defaults to `ibcmd`. `agent` is
  experimental for most rows and is reached only by naming it — except on a standalone server,
  where its SSH gate is the only executor there is.
- An override is strict: if the named executor is not ready the command refuses with a reason
  and never falls back to the default chain.
- The key is allowed in `v8project.local.yaml` too; the response receipt names the file it came from.

## Format And Backend Rules

- `format=DESIGNER`: `infobase create`, `push`, `extensions`, `pull`, Designer syntax checks, tests, and
  make/upload/artifact workflows if configured.
- `format=EDT`: `infobase create` and `push` through EDT export to Designer files, EDT syntax checks,
  extensions, and tests.
- `ibcmd` as the executor for `push` covers file infobases and server infobases with `infobase.dbms`;
  for an EDT project it runs after the EDT export to Designer files and requires a file infobase.
- `infobase.cluster` (`ras`, `user`/`password`, `agent.address`/`user`/`password`) declares the
  administration server and the two administrator levels above the infobase user; it is refused
  next to `File=` or `standalone`, and no command reads it yet (`sessions`, the runner's own `ras`
  and `infobase create` in a cluster arrive later).
- `extensions` supports Designer and EDT projects: `--name` selects an extension `source-set`,
  `--installed-name` selects an installed platform name without a matching source-set.
- `check` picks its branch by `format`: `/CheckConfig` under DESIGNER, EDT validation under EDT.
- IBCMD dump uses project-local standalone-server data under `workPath/ibcmd-data`.
- `pull --mode partial` with IBCMD degrades to incremental and must be called out in user-facing summaries.
- `convert` is CLI-only, repo-aware, uses configured `source-set`, takes no `providers` key, and does not require an infobase.
- `upload` supports `.cf` and `.cfe` only for `format=DESIGNER`.
- `tools.client_mcp.extension.source` is prepared during `push`, skipped when unchanged, and refreshed by `push --full`; `.artifact.path` must point to `.cfe` and currently needs the Designer executor.
- `make` / `artifacts` run through Designer (`agent` only via `providers.make: agent`; on a standalone server the agent is the only executor) and publish `.cf`, `.cfe`, `.epf`, or `.erf` depending on target/source-set.

## Source-Set Notes

`source-set.name` is the stable identity for ordering, diagnostics, runtime contexts, generated directories, and command selection.
Relative `source-set.path` values are resolved from the directory containing the primary `v8project.yaml`.

Supported `source-set.type` values:

- `CONFIGURATION`
- `EXTENSION`
- `EXTERNAL_DATA_PROCESSORS`
- `EXTERNAL_REPORTS`

Prefer `--source-set <NAME>` for narrow push, pull, convert, and artifact flows when the user's change is scoped to one configured source-set.

## Config Path

`v8project.yaml` is the default config filename. Use `--config <path>` only when the active project config is not at the default path or the user explicitly asks for that command form.

`v8project.local.yaml` is an automatic local overlay only. It may override only `workPath`,
`infobase.*`, `tools.*`, `tests.*`, `providers`, and `mcp.*`; it must not define `source-set` or
`format`, and it must not be used as `--config`. `--workdir` wins over both config files.
`init` creates the sibling local overlay as an empty mapping with a schema modeline and adds
`v8project.local.yaml` to `.gitignore` when needed.
