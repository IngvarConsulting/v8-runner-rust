# Documentation Guide

This repository keeps one explicit documentation stack so maintainers and agents do not have
to guess which Markdown file is authoritative.

## Source Of Truth Order

1. Current code and live CLI help.
2. Public project docs:
   - `README.md`
   - `docs/CAPABILITIES.md`
   - `docs/CONFIGURATION.md`
   - `docs/DEEP_DIVE.md`
3. Contributor module map:
   - `ARCHITECTURE.md`
4. Active internal spec and architecture docs:
   - `spec/README.md`
   - `spec/arch/rules/*` — the agreed guarantees, each naming its check
   - `spec/architecture/*`
5. Raw external 1C references and measurements taken on a live platform:
   - `references/1c/*`

## Search Hygiene

Default `rg` searches ignore the raw 1C corpus under `references/1c/` through `.rgignore`,
with one exception: `references/1c/confirmed-runtime-measurements.md` stays searchable,
because facts measured on a live platform exist nowhere else and are meant to be found.
Use `rg -uu` when you need the raw upstream command reference itself.
