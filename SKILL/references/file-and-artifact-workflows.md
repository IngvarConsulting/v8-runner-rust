# File And Artifact Workflows

Use these commands when the task is about files, artifacts, publication, or source-format conversion.

## Pull

`pull` reverse-syncs the current infobase state back to project files.

```bash
git status --short
v8-runner pull
git diff
```

Supported modes:

```bash
v8-runner pull --force
v8-runner pull
v8-runner pull --object Catalog:Items
```

Useful selectors:

```bash
v8-runner pull <NAME>
v8-runner pull --extension <EXTENSION>
```

No key is incremental, `--object` is partial, `--force` replaces the directory with a full dump (check `git status` first). When `ibcmd` executes the pull, object-scoped partial degrades to incremental with a warning.

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
v8-runner download main --output dist/main.cf
v8-runner download --state db --output dist
v8-runner download --state db --extension Sales --output dist/sales.cfe
```

Without a set and `--extension`, `--output` is a directory: the infobase is asked which extensions
it has, and every configuration/extension set is written as `<dir>/<SET>.cf` / `<dir>/<SET>.cfe`;
an extension set missing from the infobase is skipped and named in `data.not_installed`. A file
path without a set is refused with `error.next` naming the set. With `source-set: []` there is
nothing to walk, and `download --output <file.cf>` takes the main configuration as before.

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

`convert` is repo-aware conversion between Designer XML, EDT projects and `.cf`/`.cfe` packages.

```bash
v8-runner convert                      # every set into the other source format
v8-runner convert <NAME> --to package  # a set into <NAME>.cf/.cfe
v8-runner convert main.cf --to xml     # a package file into Designer XML
v8-runner convert --output <DIR>
```

It is not a `pull` alias:

- it never selects the project infobase: a project without one converts, `--infobase` is refused;
- a positional value ending in `.cf`/`.cfe` is a package file, anything else a set; a set whose name ends in `.cf`/`.cfe` is named with `--source-set <NAME>`;
- `--to xml|edt|package`; without it a set goes to the other source format and a package file to xml; Designer sets go to edt/package, EDT sets to xml/package (through xml), a package file only to xml — any other pair is refused before the lock;
- package directions run `ibcmd`: source-to-package shares the `make` build infobase, and package-to-XML uses a throwaway infobase. `data.provider` names the executor. A missing `ibcmd` returns `environment`. `providers.convert` is refused while `ibcmd` is the only executor;
- `--to package` without a set takes the configuration and extension sets; an external set is refused (use `make`);
- without `--output`, results go under `workPath/convert/out/<sourceSetName>/<designer|edt>/`, packages under `workPath/convert/out/packages/`, a package file's XML under `workPath/convert/out/from-package/<file name>/`;
- `--output` is a target root and mirrors `source-set.path` relative to the primary config directory; with `--to package` it reads like `make`: without a set a directory for `<SET>.cf|.cfe` (a file path is refused with `next`), with a set the package file itself or its directory; for a package file `--output` is the XML directory itself.

## Upload

`upload` applies existing `.cf` or `.cfe` artifacts and installs vendor updates in a supported main configuration.

```bash
v8-runner upload <FILE>
v8-runner upload <FILE> --mode combine --settings <FILE>
v8-runner upload <FILE.cfe> --ref <NAME>
v8-runner upload <VENDOR_PACKAGE> --mode update --settings <FILE>
```

Rules:

- supported only for `format=DESIGNER`, and `upload` is Designer-only;
- `.cfe` requires `--ref <NAME>`, the extension name in the base. An extension source-set
  is named by its set name, as in `push`; its `Configuration.xml` is not read.
  `--extension` remains a hidden alias.
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
- `upload --mode update` requires a vendor distribution `.cf` or update `.cfu` and
  readable merge-settings XML. It runs `/UpdateDBCfg` only after the working configuration
  reports a changed version. Use `--dry-run` to inspect the `/UpdateCfg` arguments.

## Make And Artifacts

`make` and `artifacts` are the same use case. Prefer `make` in examples unless the user uses the alias.

```bash
v8-runner make --output <DIR>
v8-runner make <NAME> --output <TARGET>
v8-runner make --output <TARGET> --extension <NAME>
```

Behavior:

- without a set and `--extension`, every set is built into `<DIR>`: `<SET>.cf`, `<SET>.cfe`, and `<SET>/` for external files, in the order configuration, extensions, external processors, reports; the first failure stops the walk; a file path is refused with `error.next` naming the main set;

- main configuration exports to `.cf`;
- extension export uses `.cfe`;
- external data processors and reports publish `.epf` / `.erf` into the output directory;
- `make` builds in `workPath/temp/build-infobase/` (Designer, then `ibcmd`). It reuses EDT exports and the loaded configuration across runs. Use `make --full` to recreate the build infobase. External `.epf`/`.erf` always use Designer. The project infobase is never selected. `providers.make` accepts `designer` and `ibcmd`. Use `pull` first to include edits made in Designer.

A full `pull` and package/external artifact publication use staged publication with backup/rollback semantics. Incremental and partial pulls are non-atomic update modes.
