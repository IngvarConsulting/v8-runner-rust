# Issue #40 Partial-Load UTF-8 BOM Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every Designer partial-load `listFile` start with exactly one UTF-8 BOM while preserving relative paths, native path separators, UTF-8 Cyrillic names, and CRLF record separators.

**Architecture:** Keep the serialization boundary in `change_detection::partial_load::write_list_file`; prefix one fixed BOM to the existing CRLF-joined UTF-8 payload and leave path validation, load planning, Designer DSL, IBCMD, and partial dump untouched. Verify the byte contract in the Rust unit module and verify platform acceptance by extending the existing trusted live Designer harness.

**Tech Stack:** Rust standard library, Cargo tests, Bash, Python 3 JSON assertions, GitHub Actions trusted Ubuntu/Windows happy-path matrix, 1C Designer.

## Global Constraints

- Write exactly one BOM byte sequence `EF BB BF` before the list payload.
- Keep entries relative to `source_root` and preserve all existing safe-path rejection behavior.
- Encode valid Cyrillic filenames as UTF-8.
- Keep native path separators: `\\` on Windows and `/` on Unix.
- Separate entries with `CRLF` (`0D 0A`) and do not append a trailing `CRLF`.
- Represent an empty payload as a BOM-only file.
- Do not change partial/full selection, BSL-to-XML expansion, Designer DSL arguments, IBCMD, or partial-dump list files.
- Add no dependency and do not introduce a general-purpose list-file serializer.
- Do not fix the unrelated Windows test-binary baseline errors in `src/use_cases/check_syntax.rs`.
- Treat `cargo test` failure on the current Windows workspace at `std::os::unix` and `Permissions::set_mode` as a pre-existing baseline limitation; use the Ubuntu contract job for executable unit-test evidence and the trusted Ubuntu/Windows happy path for real Designer evidence.

## File Map

- Modify `src/change_detection/partial_load.rs`: own the partial-load byte serialization contract, its rustdoc, and exact unit regression.
- Modify `scripts/test/live-cli-fixture.sh`: own JSON partial-mode validation and the real Designer partial-load smoke step.
- Do not modify `src/use_cases/build_project.rs`, `src/platform/designer.rs`, `src/use_cases/dump_config/**`, or `SKILL/SKILL.md`.

---

### Task 1: Serialize Designer Partial-Load Lists as UTF-8 with BOM

**Files:**
- Modify: `src/change_detection/partial_load.rs:52-63`
- Test: `src/change_detection/partial_load.rs:215-238`

**Interfaces:**
- Consumes: `pub fn relative_paths(paths: &[PathBuf], source_root: &Path) -> std::io::Result<Vec<PathBuf>>`
- Produces: unchanged `pub fn write_list_file(paths: &[PathBuf], source_root: &Path, dest: &Path) -> std::io::Result<()>`
- Produces byte contract: `EF BB BF + UTF8(paths joined by "\r\n")`

- [ ] **Step 1: Add the exact failing byte regression**

Add this test before changing production code:

```rust
#[test]
fn write_list_file_uses_utf8_bom_and_crlf_for_unicode_relative_paths() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("src");
    let first = root.join("CommonModules").join("ОбщийМодуль1.xml");
    let second = root
        .join("CommonModules")
        .join("ОбщийМодуль1")
        .join("Ext")
        .join("Module.bsl");
    let list_file = temp.path().join("partial.lst");

    std::fs::create_dir_all(first.parent().expect("first parent")).expect("first parent dir");
    std::fs::create_dir_all(second.parent().expect("second parent"))
        .expect("second parent dir");
    std::fs::write(&first, "<xml />").expect("write first");
    std::fs::write(&second, "procedure Test()\nendprocedure").expect("write second");

    write_list_file(&[first.clone(), second.clone()], &root, &list_file).expect("write list");

    let relative_payload = [first, second]
        .iter()
        .map(|path| {
            path.strip_prefix(&root)
                .expect("relative path")
                .display()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\r\n");
    let mut expected = b"\xEF\xBB\xBF".to_vec();
    expected.extend_from_slice(relative_payload.as_bytes());

    assert_eq!(std::fs::read(list_file).expect("read list"), expected);
}
```

Update `write_list_file_skips_empty_relative_paths` to assert the new empty-list contract:

```rust
assert_eq!(
    std::fs::read(list_file).expect("read list"),
    b"\xEF\xBB\xBF"
);
```

- [ ] **Step 2: Run the focused test and observe RED**

Run in an Unix-capable environment:

```bash
cargo test --locked change_detection::partial_load::tests::write_list_file_uses_utf8_bom_and_crlf_for_unicode_relative_paths -- --exact
```

Expected: FAIL because actual bytes start with the first relative path rather than `EF BB BF`.

On the current Windows workspace, record the known unrelated compile failure instead of treating it as a regression:

```text
src/use_cases/check_syntax.rs:995: cannot find `unix` in `os`
src/use_cases/check_syntax.rs:1002: no method named `set_mode`
```

- [ ] **Step 3: Implement the minimal byte serialization**

Add the BOM constant near the other module constants and replace only the final write in `write_list_file`:

```rust
const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";
```

```rust
/// Write a partial-load list file as UTF-8 with BOM and CRLF-separated paths.
///
/// Paths are written relative to `source_root` as required by Designer's
/// `-listFile` parameter when running in agent mode. Path component separators
/// remain native to the current operating system.
pub fn write_list_file(paths: &[PathBuf], source_root: &Path, dest: &Path) -> std::io::Result<()> {
    let rel_paths = relative_paths(paths, source_root)?;
    let lines = rel_paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    let contents = lines.join("\r\n");
    let mut payload = Vec::with_capacity(UTF8_BOM.len() + contents.len());
    payload.extend_from_slice(UTF8_BOM);
    payload.extend_from_slice(contents.as_bytes());
    std::fs::write(dest, payload)
}
```

- [ ] **Step 4: Run focused and safe-path tests and observe GREEN in the supported test contour**

Run:

```bash
cargo test --locked change_detection::partial_load
```

Expected on the Ubuntu contract runner: all `change_detection::partial_load` tests pass, including the exact byte regression, BOM-only empty payload, relative-path checks, and unsafe-path rejection.

Run the locally available compile check:

```bash
cargo check --locked --bin v8-runner
```

Expected: exit code 0. Do not claim local Windows unit tests passed; retain the baseline limitation in the handoff.

- [ ] **Step 5: Format, inspect, and commit Task 1**

Run:

```bash
cargo fmt --all -- --check
git diff --check
git diff -- src/change_detection/partial_load.rs
```

Expected: formatting and whitespace checks pass; the diff is limited to the BOM constant, rustdoc, writer payload, and two byte-contract tests.

Commit:

```bash
git add src/change_detection/partial_load.rs
git commit -m "fix(build): write partial-load list with UTF-8 BOM" -m "- prefix exactly one BOM before CRLF-separated relative paths
- cover Cyrillic paths and BOM-only empty payloads"
```

---

### Task 2: Exercise the BOM Contract through Real Designer Partial Build

**Files:**
- Modify: `scripts/test/live-cli-fixture.sh:182-200`
- Modify: `scripts/test/live-cli-fixture.sh:579-590`

**Interfaces:**
- Consumes JSON build result `data.steps[].{source_set, mode, ok}`.
- Produces shell helper `assert_json_step_partial <json_path> <source_set>`.
- Produces trusted smoke artifact `target/manual-tests/live-cli-designer/json/build-partial.json` unless `V8TR_LIVE_CLI_OUTPUT_ROOT` overrides the root.

- [ ] **Step 1: Add a strict partial-mode JSON assertion**

Add this helper immediately after `assert_json_step_ok`:

```bash
assert_json_step_partial() {
    local json_path="$1"
    local source_set="$2"
    python3 - "$json_path" "$source_set" <<'PY'
import json
import sys

json_path, source_set = sys.argv[1], sys.argv[2]
with open(json_path, "r", encoding="utf-8") as fh:
    payload = json.load(fh)

steps = payload.get("data", {}).get("steps", [])
for step in steps:
    if step.get("source_set") != source_set:
        continue
    mode = step.get("mode")
    partial = mode.get("partial") if isinstance(mode, dict) else None
    file_count = partial.get("file_count") if isinstance(partial, dict) else None
    if step.get("ok") is not True:
        raise SystemExit(f"partial build step for '{source_set}' is not successful: {step}")
    if not isinstance(file_count, int) or file_count < 1:
        raise SystemExit(
            f"build step for '{source_set}' is not partial with a positive file count: {step}"
        )
    raise SystemExit(0)

raise SystemExit(f"build output does not contain step for '{source_set}'")
PY
}
```

- [ ] **Step 2: Add the real partial-load smoke after the no-op build**

Insert after the existing incremental no-op assertions and before the extensions stage:

```bash
partial_source="$WORK_BASE_PATH/$CONFIGURATION_SOURCE_SET_PATH/CommonModules/ОбщийМодуль1/Ext/Module.bsl"
assert_file_exists "$partial_source"
printf '\n// issue-40 partial-load BOM smoke\n' >> "$partial_source"

partial_build_json="$OUTPUT_ROOT/json/build-partial.json"
print_stage "build partial after Cyrillic source change"
run_cli_json_to_file \
    "$partial_build_json" \
    build --source-set "$CONFIGURATION_SOURCE_SET_NAME"
assert_json_step_ok "$partial_build_json" "$CONFIGURATION_SOURCE_SET_NAME"
assert_json_step_partial "$partial_build_json" "$CONFIGURATION_SOURCE_SET_NAME"
```

This uses the copied workspace fixture, never edits `tests/fixtures/designer`, and forces Designer to consume a partial-load list containing the Cyrillic `ОбщийМодуль1` path.

- [ ] **Step 3: Validate the shell contract locally**

Run:

```bash
bash -n scripts/test/live-cli-fixture.sh
git diff --check
```

Expected: both commands exit 0.

Review the diff and confirm all variable expansions containing paths are double-quoted and the new stage is after full build plus no-op:

```bash
git diff -- scripts/test/live-cli-fixture.sh
```

- [ ] **Step 4: Run the trusted live Designer matrix**

Run through the existing happy-path workflow on trusted Ubuntu and Windows jobs:

```bash
V8_RUNNER_CI_SCOPE=happy-path bash scripts/test/ci-rust.sh
```

Expected in each trusted environment:

- `build partial after Cyrillic source change` exits 0;
- `build-partial.json` contains an `ok: true` step for the configuration source-set;
- that step serializes `mode` as `{"partial":{"file_count":N}}` with `N >= 1`;
- Designer accepts the generated `-listFile` on both Ubuntu and Windows.

For fork PRs without platform secrets, expect the established live soft-skip; acceptance requires the trusted upstream matrix before merge.

- [ ] **Step 5: Commit Task 2**

```bash
git add scripts/test/live-cli-fixture.sh
git commit -m "test(build): exercise Designer partial-load list" -m "- trigger a partial build from a Cyrillic BSL path
- require a successful partial mode in trusted live smoke"
```

---

## Final Verification and Review Gates

- [ ] Run `cargo fmt --all -- --check`; expect exit code 0.
- [ ] Run `cargo check --locked --bin v8-runner`; expect exit code 0.
- [ ] Run `bash -n scripts/test/live-cli-fixture.sh`; expect exit code 0.
- [ ] Run `git diff --check upstream/master...HEAD`; expect exit code 0.
- [ ] Run the focused Rust test on Ubuntu and record the exact pass count.
- [ ] Run or inspect the trusted Ubuntu/Windows happy-path jobs and record both results.
- [ ] Record the unrelated local Windows `cargo test` baseline failure without claiming the suite passed locally.
- [ ] Dispatch the repository-required independent Tests/Tester subagent to rerun available checks.
- [ ] Dispatch the repository-required Reviewer subagent to inspect scope, cross-platform byte handling, shell quoting, and acceptance coverage.
- [ ] Apply the repository-required `/rust-expert-best-practices-code-review` checklist independently; if the skill remains unavailable in the environment, report that limitation explicitly and perform an equivalent local Rust safety/API/error-handling review rather than silently omitting it.
- [ ] Confirm `SKILL/SKILL.md`, IBCMD, Designer DSL, partial dump, and unrelated files remain unchanged.
- [ ] Confirm `git status --short` is clean after the implementation commits.
