# Project Workflows

Use these flows by user intent. Do not split the workflow only because source files are Designer or EDT; many commands share the same lifecycle and differ only by `format`, the executor chosen per operation, or tool availability.

For exact support rules, read `config-and-backends.md` together with this file.

## Setup

Create the default config when the project has no `v8project.yaml`:

```bash
v8-runner init
```

This creates `v8project.yaml`, a sibling empty `v8project.local.yaml` for machine-local overrides,
and a `.gitignore` entry for the local overlay when needed.

Choose a narrower `init` command only when the project shape is known:

```bash
v8-runner init --connection "File=build/ib"
v8-runner init --format edt
```

Create a new project from an existing infobase when the infobase is the current source of truth:

```bash
v8-runner clone --connection "File=/path/to/ib" --platform-version 8.3.27
```

`clone` creates config/local overlay/gitignore, dumps the main configuration to
`src/configuration`, and stores credentials only in `v8project.local.yaml` when `--user` or
`--password` is passed. It does not auto-discover extensions.

Initialize generated runtime state only when the file infobase or EDT workspace needs to be created:

```bash
v8-runner infobase create
```

## Push

Apply Git-visible source changes to the configured runtime state:

```bash
v8-runner push
```

Use a full push after branch switches, rebases, broad object moves, or suspicious incremental state:

```bash
v8-runner push --full
```

`push` is a common workflow. For EDT projects it may export EDT sources to Designer files before applying them through the configured backend. For Designer projects it applies Designer sources directly through the configured backend.

If `tools.client_mcp.extension` is configured, `push` also prepares that tool extension after the project source-set stage, including scoped `--source-set` builds. Source-backed tool extensions use their own change-detection state and are skipped when unchanged; use `push --full` to force refresh. Do not add a tool extension as a project `source-set` or select it with `--source-set`.

## Syntax Checks

Choose syntax checks from config capabilities, not from assumptions about the repository name.

Designer module checks:

```bash
v8-runner push
v8-runner check designer-modules --server --thin-client
```

Designer configuration checks:

```bash
v8-runner push
v8-runner check designer-config
```

EDT checks:

```bash
v8-runner push
v8-runner check edt
```

If a `check` command is unavailable for the current `format`, report the config limitation instead of inventing raw platform commands.

## Pull

Use `pull` when the desired source of truth is the current infobase state.

Before pulling, inspect current Git changes:

```bash
git status --short
```

Incremental pull:

```bash
v8-runner pull --mode incremental
```

Partial object pull when the backend supports it:

```bash
v8-runner pull --mode partial --object <TYPE:NAME>
```

Run `git diff` after `pull` and report the affected files.

## Extensions

Use `extensions` when extension properties need to be synchronized without a broader recovery step.

Do not replace extension-specific synchronization with a full rebuild unless the user asks for recovery or the narrower command fails for a relevant reason.

```bash
v8-runner extensions
v8-runner extensions --name <SOURCE_SET>
v8-runner extensions --installed-name YAXUNIT --dry-run
v8-runner extensions --installed-name YAXUNIT
```

## Launch

Prefer runner launch commands over raw `1cv8` command construction:

```bash
v8-runner launch designer
v8-runner launch thin
v8-runner launch thick
v8-runner launch ordinary
```

Check what a launch would run before running it; the preview never spawns the client:

```bash
v8-runner --json-message launch thin --dry-run
```

Launch onec-client-mcp-devkit through the supported `launch mcp` surface instead of manually assembling `/C runMcp...`:

```bash
v8-runner launch mcp
v8-runner launch mcp --mode thin --mcp-port <PORT>
v8-runner launch mcp --mcp-config <FILE>
v8-runner launch mcp --mcp-port <PORT> --wait-ready
```

For ordinary direct launches, typed launch flags include `--c`, `--execute`, `--use-privileged-mode`, `--output`, and repeatable `--raw-key`.

For `launch mcp`, use `--mcp-config` and `--mcp-port`; do not pass `/C` through `--c`.
Use `--wait-ready` when a following agent or tool needs the client MCP HTTP endpoint to be initialized and able to return `tools/list`.

`launch mcp` and `launch mcp va` do not install or update `tools.client_mcp.extension`; run `v8-runner push` first when that extension may be missing or stale.

For `launch mcp va`, read `testing.md`; it is part of the Vanessa Automation debugging and scenario-authoring workflow.
