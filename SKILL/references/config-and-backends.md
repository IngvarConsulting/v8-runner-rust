# Config And Backends

Inspect `v8project.yaml` before diagnosing build, syntax, dump, test, and launch behavior.
If a sibling `v8project.local.yaml` exists, inspect it too because it overrides machine-local
settings before CLI overrides.

## Fields To Check First

- `workPath`: generated state, temp files, and workspace location.
- `format`: `DESIGNER` or `EDT`.
- `providers.<operation>`: optional per-operation executor override. Omit it to use the defaults below.
- `infobase.connection`: often `File=build/ib` for local automation.
- `source-set`: ordered configuration and extension sources.
- `tools.platform.path`, `version`, and `strict`: platform discovery hints. `path` is always an
  explicit-only boundary with no default-root or `PATH` fallback. Without `path`, `version` filters
  normal discovery. With `path`, `version` is ignored unless `strict: true`; strict path+version
  resolution rejects unknown or mismatched versions and pins sibling utilities to one canonical root.
- `tools.edt_cli.path`, `version`, and `interactive-mode`: EDT CLI discovery and execution mode.
- `tests.yaxunit` and `tests.va`: test runner configuration.
- `tools.client_mcp`, `tools.va`, and `tools.enterprise`: launch and client-side MCP integration hints.
- `tools.client_mcp.extension`: optional tool extension prepared by `build`; it is not a project `source-set`.
- `tools.client_mcp.wait_ready_timeout_ms`: optional readiness timeout for `launch mcp --wait-ready`; falls back to `execution_timeout` and is still capped by the command deadline.

## Choosing The Executor

There is no global backend switch. The executor is chosen per operation from a capability
matrix, and `providers.<operation>` names one explicitly.

- Valid keys: `init`, `build`, `load`, `dump`, `extensions`, `infobase.configuration.export`,
  `infobase.dump`, `infobase.restore`, `syntax`, `make`.
- Defaults: `init`, `build`, `dump`, `infobase.configuration.export` try Designer then `ibcmd`;
  `infobase.dump` and `infobase.restore` use Designer (experimental IBCMD DT only when named);
  `load`, `syntax`, `make` are Designer-only; `extensions` is `ibcmd`-only.
- The key is accepted only for an operation that has a real choice on this target; naming an
  executor for a single-executor operation is a config error.
- An override is strict: if the named executor is not ready the command refuses with a reason
  and never falls back to the default chain.
- The key is allowed in `v8project.local.yaml` too; the response receipt names the file it came from.

## Format And Backend Rules

- `format=DESIGNER`: init, build, extensions, dump, Designer syntax checks, tests, and
  make/load/artifact workflows if configured.
- `format=EDT`: init and build through EDT export to Designer files, EDT syntax checks,
  extensions, and tests.
- `ibcmd` as the executor for `build` covers file infobases and server infobases with `infobase.dbms`;
  for an EDT project it runs after the EDT export to Designer files and requires a file infobase.
- `extensions` supports Designer and EDT projects, but only extension `source-set` entries are actionable.
- `syntax designer-config` and `syntax designer-modules` require `format=DESIGNER`; `syntax edt` requires `format=EDT`.
- IBCMD dump uses project-local standalone-server data under `workPath/ibcmd-data`.
- `dump --mode partial` with IBCMD degrades to incremental dump and must be called out in user-facing summaries.
- `convert` is CLI-only, repo-aware, uses configured `source-set`, takes no `providers` key, and does not require an infobase.
- `load` supports `.cf` and `.cfe` only for `format=DESIGNER`.
- `tools.client_mcp.extension.source` is prepared during `build`, skipped when unchanged, and refreshed by `build --full-rebuild`; `.artifact.path` must point to `.cfe` and currently needs the Designer executor.
- `make` / `artifacts` are Designer-only and publish `.cf`, `.cfe`, `.epf`, or `.erf` depending on target/source-set.

## Source-Set Notes

`source-set.name` is the stable identity for ordering, diagnostics, runtime contexts, generated directories, and command selection.
Relative `source-set.path` values are resolved from the directory containing the primary `v8project.yaml`.

Supported `source-set.type` values:

- `CONFIGURATION`
- `EXTENSION`
- `EXTERNAL_DATA_PROCESSORS`
- `EXTERNAL_REPORTS`

Prefer `--source-set <NAME>` for narrow build, dump, convert, and artifact flows when the user's change is scoped to one configured source-set.

## Config Path

`v8project.yaml` is the default config filename. Use `--config <path>` only when the active project config is not at the default path or the user explicitly asks for that command form.

`v8project.local.yaml` is an automatic local overlay only. It may override only `workPath`,
`infobase.*`, `tools.*`, `tests.*`, `providers`, and `mcp.*`; it must not define `source-set` or
`format`, and it must not be used as `--config`. `--workdir` wins over both config files.
`config init` creates the sibling local overlay as an empty mapping with a schema modeline and adds
`v8project.local.yaml` to `.gitignore` when needed.
