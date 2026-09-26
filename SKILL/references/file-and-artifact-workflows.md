# File And Artifact Workflows

Use these commands when the task is about files, artifacts, publication, or source-format conversion.

## Pull

`pull` reverse-syncs the current infobase state back to project files.

```bash
git status --short
v8-runner pull --mode incremental
git diff
```

Supported modes:

```bash
v8-runner pull --mode full
v8-runner pull --mode incremental
v8-runner pull --mode partial --object Catalog:Items
```

Useful selectors:

```bash
v8-runner pull --mode incremental --source-set <NAME>
v8-runner pull --mode incremental --extension <EXTENSION>
```

`partial` requires at least one `--object`. When `ibcmd` executes the pull, object-scoped partial degrades to incremental with a warning.

Use `TYPE:NAME` as the canonical partial selector form, for example `Catalog:Items`.
The dotted `TYPE.NAME` form remains compatible. The Designer list and JSON
`data.selectors[*].normalized` use `TYPE.NAME`; JSON `data.selectors[*].requested`
preserves the submitted selector. Before the platform starts, selector syntax is validated:
`TYPE` and `NAME` must be non-empty, exactly one `:` or `.` separator is required, and
control characters are rejected. When Designer executes the pull it validates whether the
metadata root type exists; when `ibcmd` does, the object list is not used because partial
degrades to incremental.

For `format=EDT`, `pull` uses an internal Designer snapshot under `workPath/designer/<sourceSetName>`, then imports the result into the EDT target.

## Export From An Infobase

Use configuration export when the artifact must reflect state already stored in the infobase:

```bash
v8-runner download --state working --output dist/main.cf
v8-runner download --state database --output dist/main.cf
v8-runner download --state database --extension Sales --output dist/sales.cfe
```

Append `--dry-run` when an orchestrator needs the selected provider and compact output plan before
apply. Preview does not create `workPath`, locks, staging/output paths, or a provider process.

These commands accept `source-set: []` because project source trees are not inputs. If no config
exists and sources are not requested, create the minimal infobase-only config described by the
skill entrypoint instead of running `clone`.

This differs from `make`: `make` builds from project sources, while this command reads the
working or database configuration from the configured infobase. The executor comes from the
matrix or from `providers.download`; runner may choose a ready alternate
during pure preflight, but never after dispatch.

Use a DT only as a portable full-infobase image, not as a backup:

```bash
v8-runner infobase dump --output dist/base.dt
```

DT export uses the implemented Designer adapter. IBCMD DT remains experimental until a
no-active-connections or exclusive-access preflight is implemented and proved; if Designer is
ready, runner selects it even when `providers.infobase.dump: ibcmd` names the other one.

Load a DT image back with the paired command, stating which irreversible change is allowed:

```bash
v8-runner infobase restore --input dist/base.dt --replace
v8-runner infobase restore --input dist/base.dt --create
```

Exactly one mode is required, and a mode that does not match the observed target is refused
before the platform starts: `--create` over an existing infobase and `--replace` over an absent
one are both `invalid_argument`. Neither provider asks on its own — Designer creates an absent
infobase and overwrites a present one — and unlike an export there is no staging step, so the
mode is the only protection the caller gets. A restore that fails after the provider started
reports `target_state: uncertain`, because how much data it had already replaced is not
observable; a refusal before the start leaves `unchanged`. Restore shares the export provider
posture: Designer `/RestoreIB` is implemented and live-verified, IBCMD `infobase restore` stays
experimental. Terminating active sessions is not exposed yet, so a busy infobase fails with the
platform's own error. The load is a critical phase: Ctrl+C or SIGTERM does not stop it — the
runner waits for the platform and reports the deferred interruption in
`execution.interruptions` with `phase: provider_command`.

## Convert

`convert` is repo-aware file conversion between Designer and EDT source formats.

```bash
v8-runner convert
v8-runner convert --source-set <NAME>
v8-runner convert --output <DIR>
```

It is not a `pull` alias:

- it does not use an infobase;
- it takes no `providers` key;
- direction is derived from configured `format`;
- without `--output`, results are published under `workPath/convert/out/<sourceSetName>/<designer|edt>/`;
- `--output` is a target root and mirrors `source-set.path` relative to the primary config directory.

`convert` is a CLI file workflow and does not run through an infobase.

## Upload

`upload` applies existing `.cf` or `.cfe` artifacts to an infobase.

```bash
v8-runner upload --path <FILE>
v8-runner upload --path <FILE> --mode combine --settings <FILE>
v8-runner upload --path <FILE> --extension <NAME>
```

Rules:

- supported only for `format=DESIGNER`, and `upload` is Designer-only;
- `.cfe` requires `--extension`;
- `--mode combine` requires `--settings`;
- use `--mode load` for the first installation of an extension; JSON reports
  `compatibility_state: "absent"` when the infobase does not list it yet, read from
  `ibcmd config extension list` rather than from any platform message;
- `--mode combine` of a configuration needs `--vendor-name <NAME>`: the platform will not
  compare a configuration with its vendor counterpart unnamed, so the state stays
  `not_probed` and the combine is refused. `--mode load` needs no name;
- `not_established` permits nothing — neither a load nor a combine. An authorization
  failure, an unreachable infobase and an unreadable extension list all land here,
  because the platform reports them with a non-zero exit code;
- `upload --mode update` is rejected by the current command contract.

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
- `make` runs through Designer; select `agent` via `providers.make: agent`, except on a standalone server, where it is the only executor.

A full `pull` and package/external artifact publication use staged publication with backup/rollback semantics. Incremental and partial pulls are non-atomic update modes.
