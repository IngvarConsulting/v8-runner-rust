# Spec Guide

`spec/` stores the active internal truth layer for planning, architecture rules, ADRs, and
acceptance.

## Active Entry Points

- `IMPLEMENTATION_TODO.md`: open implementation tasks only.
- `ADR_DERIVED_BACKLOG.md`: open ADR-derived gaps that still need planning or execution.
- `decisions/README.md`: accepted architecture decisions and their owning ADR files.
- `arch/`: атомарный реестр решений, инвариантов и контрактов; читать с `arch/index.md`.
- `archive/FATE.md`: судьба записей замороженного прежнего слоя.
- `architecture/change-checklist.md`: required sync/checklist for contract and boundary changes.
- `architecture/arc42/`: detailed architecture and risk set for maintainers.
- `acceptance/real-environment-validation.md`: active real-environment acceptance and smoke plan.

## Archive

- Historical snapshots and closed delivery records live in `spec/archive/`.
- Raw external 1C references live in `references/1c/`.

## Usage Rule

If a statement here conflicts with current code, CLI help, or the public docs layer, trust the
current code first and then update the active doc layer.
