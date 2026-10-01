# Documentation Guide

This repository keeps one explicit documentation stack so maintainers and agents do not have
to guess which Markdown file is authoritative.

## Which Source Answers What

- **What the program does**: current code and live CLI help.
- **What must keep holding**: the rules in `spec/rules/` — each names its check, or a
  `gap` issue while it is not yet fulfilled.
- **How to use it**: the public docs (`README.md`, `docs/CAPABILITIES.md`,
  `docs/CONFIGURATION.md`, `docs/DEEP_DIVE.md`). Users read them as the contract, so when they
  disagree with the code, the gap is either a defect or a public-contract change — decided as
  "When a Rule and the Code Disagree" in `AGENTS.md` describes — never a silent doc edit.
- **How it is built**: `spec/arc42/`; its section 5 is the only module map. It describes
  and promises nothing; when it disagrees with the code, it is updated.
- **Raw external 1C references and measurements taken on a live platform**:
  `references/1c/*`.

When the code and a rule disagree, the code is not automatically right: see "When a Rule and
the Code Disagree" in [`AGENTS.md`](../AGENTS.md). How an agent reaches each source is mapped
in [`AI_DEV.md`](../AI_DEV.md).

## Search Hygiene

Default `rg` searches ignore the raw 1C corpus under `references/1c/` through `.rgignore`,
with one exception: `references/1c/confirmed-runtime-measurements.md` stays searchable,
because facts measured on a live platform exist nowhere else and are meant to be found.
Use `rg -uu` when you need the raw upstream command reference itself.
