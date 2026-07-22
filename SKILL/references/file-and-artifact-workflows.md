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
v8-runner dump --mode partial --object <TYPE:NAME>
```

Useful selectors:

```bash
v8-runner dump --mode incremental --source-set <NAME>
v8-runner dump --mode incremental --extension <EXTENSION>
```

`partial` requires at least one `--object`. With `builder=IBCMD`, object-scoped partial dump degrades to incremental dump with a warning.

For `format=EDT`, dump uses an internal Designer snapshot under `workPath/designer/<sourceSetName>`, then imports the result into the EDT target.

Dump safety and recovery rules:

- each infobase/source-set identity owns private shadows and generation-scoped baselines under `workPath`; never reuse them across infobases;
- incremental and partial dump require a valid matching baseline and private `ConfigDumpInfo.xml`; missing or corrupt state promotes that one operation to a full dump;
- the receipt is an exact manifest delta: `requested` records every add/modify/delete and `processed`, `skipped`, or `conflicted` records its outcome with byte hashes;
- a three-way conflict publishes no project-source files or new private generation;
- full, incremental, and partial modes all use recoverable manifest publication. On interruption, restart recovery either completes the exact dump transaction or restores the previous managed file state without treating an unrelated same-numbered generation as success.

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

Package and external-artifact publication uses staged backup/rollback semantics. Every dump mode uses journaled, manifest-scoped source publication and private-state recovery.
