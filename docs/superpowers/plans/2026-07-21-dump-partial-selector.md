# Partial dump selector normalization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make documented `TYPE:NAME` partial-dump selectors valid for Designer while preserving existing `TYPE.NAME` input.

**Architecture:** Parse raw `DumpRequest.objects` into a domain selector at the dump boundary. The selector retains the request and renders one canonical Designer value; only canonical values go to the list file and requested/normalized pairs go to `DumpResult` JSON.

**Tech Stack:** Rust, serde, clap, unit tests in `src/use_cases/dump_config.rs`, CLI integration tests, Markdown documentation.

## Global Constraints

- `TYPE:NAME` is canonical; `TYPE.NAME` is documented compatibility input.
- Reject blank segments, control characters, extra separators, and unsupported metadata root types before spawning Designer.
- Preserve selector order and report requested and normalized values in JSON.
- Use `Result`/`AppError::Validation`, borrowed `&str`/`&[T]`, and exhaustive enum matches; no production panic/unwrap for input errors.
- Do not change IBCMD fallback, source-set selection, or Designer command grammar.

---

### Task 1: Parse and carry canonical partial-dump selectors

**Files:**
- Create: `src/domain/partial_dump_selector.rs`
- Modify: `src/domain/mod.rs`, `src/domain/dump.rs`
- Modify: `src/use_cases/dump_config/helpers.rs`, `src/use_cases/dump_config/coordinator.rs`, `src/use_cases/dump_config.rs`

**Interfaces:**
- `PartialDumpSelector::parse(requested: &str) -> Result<PartialDumpSelector, AppError>`.
- `PartialDumpSelector::requested(&self) -> &str` and `normalized(&self) -> String`.
- `DumpSelectorResult { requested: String, normalized: String }`; `DumpResult.selectors: Option<Vec<DumpSelectorResult>>`.
- List-file helpers accept `&[PartialDumpSelector]` and write `normalized()` one per line.

- [ ] **Step 1: Write failing domain and use-case tests**

Test `Catalog:Items -> Catalog.Items`, dotted compatibility, `Unknown:Items`, `Catalog:`, `Catalog:Items:Extra`, and `Catalog:Item\nInjected`. Change the captured Designer-list test to submit `Catalog:Items` and `Document:Order`, assert exactly `Catalog.Items\nDocument.Order\n`, and assert ordered result pairs:

```rust
assert_eq!(result.selectors.as_deref(), Some(&[
    DumpSelectorResult {
        requested: "Catalog:Items".to_owned(),
        normalized: "Catalog.Items".to_owned(),
    },
]));
```

- [ ] **Step 2: Run RED tests**

Run `cargo test --locked partial_dump_selector --lib` and `cargo test --locked dump_partial_designer --lib`. Expect failure because the typed parser/result field do not exist and colon values are copied verbatim.

- [ ] **Step 3: Implement minimal typed parser and result propagation**

Create `MetadataRootType` with exhaustive variants for: `AccountingRegister`, `AccumulationRegister`, `Bot`, `BusinessProcess`, `CalculationRegister`, `Catalog`, `ChartOfAccounts`, `ChartOfCalculationTypes`, `CommonAttribute`, `CommonCommand`, `CommonForm`, `CommonModule`, `Constant`, `DataProcessor`, `DefinedType`, `Document`, `DocumentJournal`, `Enum`, `EventSubscription`, `ExchangePlan`, `FilterCriterion`, `FunctionalOption`, `FunctionalOptionsParameter`, `HTTPService`, `InformationRegister`, `IntegrationService`, `Language`, `Report`, `Role`, `ScheduledJob`, `Sequence`, `SessionParameter`, `Style`, `Subsystem`, `Task`, `WebService`, `WSReference`, and `XDTOPackage`.

Split exactly once on `:` or `.`; reject zero or multiple separators, unknown root type, empty trimmed name, or any control character. Store outer-trimmed request and render `<type>.<name>`. Replace validated raw objects with `Vec<PartialDumpSelector>`, use it to write the Designer file, and map partial-mode selectors to `DumpSelectorResult`.

- [ ] **Step 4: Run GREEN tests**

Run `cargo test --locked partial_dump_selector --lib`, `cargo test --locked dump_partial_designer --lib`, and `cargo fmt --check`; all selected tests and formatting must pass.

- [ ] **Step 5: Commit Task 1**

Run `git add src/domain/partial_dump_selector.rs src/domain/mod.rs src/domain/dump.rs src/use_cases/dump_config/helpers.rs src/use_cases/dump_config/coordinator.rs src/use_cases/dump_config.rs` then `git commit -m "fix(dump): normalize partial object selectors"`.

### Task 2: Expose and document the public contract

**Files:**
- Modify: `tests/cli_dump.rs`, `docs/CAPABILITIES.md`, `SKILL/references/file-and-artifact-workflows.md`

**Interfaces:**
- JSON output has `data.selectors[*].requested` and `data.selectors[*].normalized`.
- Docs present colon input first and dotted input as compatibility.

- [ ] **Step 1: Write failing CLI JSON regression test**

Add a `--json-message dump --mode partial --object Catalog:Items` test with a stub Designer. Assert list line `Catalog.Items` and:

```rust
assert_eq!(payload["data"]["selectors"][0]["requested"], "Catalog:Items");
assert_eq!(payload["data"]["selectors"][0]["normalized"], "Catalog.Items");
```

- [ ] **Step 2: Run RED test**

Run the exact test with `cargo test --locked --test cli_dump <test-name>`. Before Task 1 it must fail because JSON lacks `selectors` and the list file has a colon.

- [ ] **Step 3: Update documentation**

In both docs files, state colon normalization, dotted compatibility, and pre-Designer rejection of invalid root types, empty names, extra separators, and controls.

- [ ] **Step 4: Verify Task 2**

Run the exact CLI test and `git diff --check`; both exit zero.

- [ ] **Step 5: Commit Task 2**

Run `git add tests/cli_dump.rs docs/CAPABILITIES.md SKILL/references/file-and-artifact-workflows.md` then `git commit -m "docs(dump): clarify partial selector syntax"`.

## Final verification

- [ ] Run focused library and CLI regressions.
- [ ] Run `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, and `cargo test --locked`.
- [ ] Record known macOS baseline failures separately if they remain; dump-focused tests must not regress.
- [ ] Run an independent Rust review for the complete branch diff and fix every Critical or Important finding, or record a waiver.
