# File And Artifact Workflows

Use these commands when the task is about files, artifacts, publication, or source-format conversion.

## Dump

`dump` reverse-syncs the current infobase state back to project files.

```bash
git status --short
v8-runner dump --mode incremental
git diff
```

Supported modes:

```bash
v8-runner dump --mode full
v8-runner dump --mode incremental
v8-runner dump --mode partial --object Catalog:Items
```

Useful selectors:

```bash
v8-runner dump --mode incremental --source-set <NAME>
v8-runner dump --mode incremental --extension <EXTENSION>
```

`partial` requires at least one `--object`. With `builder=IBCMD`, object-scoped partial dump degrades to incremental dump with a warning.

Use `TYPE:NAME` as the canonical partial selector form, for example `Catalog:Items`.
The dotted `TYPE.NAME` form remains compatible. The Designer list and JSON
`data.selectors[*].normalized` use `TYPE.NAME`; JSON `data.selectors[*].requested`
preserves the submitted selector. Before the platform starts, selector syntax is validated:
`TYPE` and `NAME` must be non-empty, exactly one `:` or `.` separator is required, and
control characters are rejected. With `builder=DESIGNER`, Designer validates whether the
metadata root type exists. With `builder=IBCMD`, the object list is not used because partial
degrades to incremental.

For `format=EDT`, dump uses an internal Designer snapshot under `workPath/designer/<sourceSetName>`, then imports the result into the EDT target.

## Export From An Infobase

Use configuration export when the artifact must reflect state already stored in the infobase:

```bash
v8-runner infobase configuration export --state working --output dist/main.cf
v8-runner infobase configuration export --state database --output dist/main.cf
v8-runner infobase configuration export --state database --extension Sales --output dist/sales.cfe
```

These commands accept `source-set: []` because project source trees are not inputs. If no config
exists and sources are not requested, create the minimal infobase-only config described by the
skill entrypoint instead of running `bootstrap`.

This differs from `make`: `make` builds from project sources, while this command reads the
working or database configuration from the configured infobase. The configured `builder` strictly
selects Designer batch or IBCMD; never retry another provider after dispatch.

Use a DT only as a portable full-infobase image, not as a backup:

```bash
v8-runner infobase dump --output dist/base.dt
```

DT export is verified only for `builder=DESIGNER`. For `builder=IBCMD`, the runner reports
`unverified` and does not dispatch until a no-active-connections or exclusive-access preflight is
implemented and proved.

## Convert

`convert` is repo-aware file conversion between Designer and EDT source formats.

```bash
v8-runner convert
v8-runner convert --source-set <NAME>
v8-runner convert --output <DIR>
```

It is not a dump alias:

- it does not use an infobase;
- it does not use `builder`;
- direction is derived from configured `format`;
- without `--output`, results are published under `workPath/convert/out/<sourceSetName>/<designer|edt>/`;
- `--output` is a target root and mirrors `source-set.path` relative to the primary config directory.

`convert` is a CLI file workflow and does not run through an infobase.

## Load

`load` applies existing `.cf` or `.cfe` artifacts to an infobase.

```bash
v8-runner load --path <FILE>
v8-runner load --path <FILE> --mode merge --settings <FILE>
v8-runner load --path <FILE> --extension <NAME>
```

Rules:

- supported only for `format=DESIGNER`, `builder=DESIGNER`;
- `.cfe` requires `--extension`;
- `--mode merge` requires `--settings`;
- `load --mode update` is rejected by the current command contract.

## Make And Artifacts

`make` and `artifacts` are the same use case. Prefer `make` in examples unless the user uses the alias.

```bash
v8-runner make --output <TARGET>
v8-runner make --output <TARGET> --source-set <NAME>
v8-runner make --output <TARGET> --extension <NAME>
```

Behavior:

- main configuration exports to `.cf`;
- extension export uses `.cfe`;
- external data processors and reports publish `.epf` / `.erf` into the output directory;
- `builder=DESIGNER` is required.

Full dump and package/external artifact publication use staged publication with backup/rollback semantics. Incremental and partial dump are non-atomic update modes.
