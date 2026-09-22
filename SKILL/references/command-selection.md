# Command Selection

Choose commands by user intent, not by listing every CLI surface.

## Setup

Use these when a project is missing `v8project.yaml` or generated runtime state:

```bash
v8-runner clone --connection "File=/path/to/ib" --platform-version 8.3.27
v8-runner init
v8-runner init --connection "File=build/ib"
v8-runner init --format edt
v8-runner infobase create
```

Use `clone` when an existing infobase is the source of truth and source files need to be
materialized. Use `init` when supported source files are already present.

Inspect `v8project.yaml` after `init` and before commands that create or mutate infobases, workspaces, or source files.

## Push And Recovery

Apply Git-visible source changes to the configured infobase:

```bash
v8-runner push
```

Limit `push` to one configured source-set:

```bash
v8-runner push --source-set <NAME>
```

Recover after branch switches, rebases, large object moves, or suspicious incremental state:

```bash
v8-runner push --full
```

Use `test` directly when behavior matters; test commands perform `push` first.

## Syntax Checks

Designer modules:

```bash
v8-runner push
v8-runner check --server --thin-client
```

Designer configuration:

```bash
v8-runner push
v8-runner check
```

EDT:

```bash
v8-runner push
v8-runner check
```

## Tests

All YaXUnit tests:

```bash
v8-runner test yaxunit all
```

Targeted YaXUnit module:

```bash
v8-runner test yaxunit module <MODULE_NAME>
```

Vanessa Automation:

```bash
v8-runner test va
```

Use Vanessa Automation for functional `.feature` acceptance scenarios: `v8-runner test va` for
configured runs, MCP `run_all_tests` with `runner: "vanessa"` for agent-driven runs, or
`launch mcp va --mcp-port <PORT> --wait-ready` for interactive debugging. If
`tools.client_mcp.port` is configured, the explicit port can be omitted. Do not use bare
`launch mcp` for these workflows.

Interactive VA debugging and scenario authoring:

```bash
v8-runner launch mcp va --mcp-port <PORT> --wait-ready
```

## Extensions

Update all configured extension properties:

```bash
v8-runner extensions
```

Update selected extension source-sets:

```bash
v8-runner extensions --name <SOURCE_SET>
```

Configure an independently installed CFE, or combine it with a configured extension:

```bash
v8-runner extensions --installed-name YAXUNIT --dry-run
v8-runner extensions --name TESTS --installed-name YAXUNIT
```

Apply disables safe mode and unsafe action protection. Selectors are repeatable; only
explicit targets run when either is supplied. Unknown `--name` remains an error.
Preview does not establish whether the extension is installed; apply reports platform failures.

## Pull, Convert, Upload, And Artifacts

Bring infobase changes back into Git-visible files:

```bash
git status --short
v8-runner pull --mode incremental
git diff
```

Pull specific objects when the backend supports it:

```bash
v8-runner pull --mode partial --object <TYPE:NAME>
```

Convert configured source-sets between Designer and EDT file formats:

```bash
v8-runner convert
v8-runner convert --source-set <NAME>
v8-runner convert --output <DIR>
```

Apply built `.cf` or `.cfe` artifacts:

```bash
v8-runner upload --path <FILE>
v8-runner upload --path <FILE> --mode combine --settings <FILE>
v8-runner upload --path <FILE> --extension <NAME>
```

Export release artifacts or publish external artifacts:

```bash
v8-runner make --output <TARGET>
v8-runner make --output <TARGET> --source-set <NAME>
v8-runner make --output <TARGET> --extension <NAME>
```

`artifacts` is a visible alias for `make`.

## Launch

Launch 1C clients through the runner:

```bash
v8-runner launch designer
v8-runner launch thin
v8-runner launch thick
v8-runner launch ordinary
```

Inspect a launch without starting a client. The preview reports the selected program and the
composed arguments with credential values replaced by `***`:

```bash
v8-runner --json-message launch thin --dry-run
```

Launch onec-client-mcp-devkit inside 1C without VA:

```bash
v8-runner launch mcp
v8-runner launch mcp --mode thin --mcp-port <PORT>
v8-runner launch mcp --mcp-config <FILE>
```
