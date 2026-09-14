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
   - `spec/arch/*`
   - `spec/architecture/*`
   - `spec/acceptance/*`
5. Historical notes and closed plans:
   - `spec/archive/*`
6. Raw external 1C references:
   - `references/1c/*`

## Search Hygiene

Default `rg` searches ignore `spec/archive/` and `references/1c/` through `.rgignore`.
Use `rg -uu` only when you intentionally need archived history or raw upstream references.
