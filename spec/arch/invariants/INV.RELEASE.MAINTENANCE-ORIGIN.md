---
id: INV.RELEASE.MAINTENANCE-ORIGIN
status: active
governs: product
decision: DEC.2026-09-22.MAINTENANCE-RELEASE-ORIGIN
check: tests/release_governance.py
scope: [release, ci]
---

# Maintenance-артефакт проверяется против точного защищённого ref

Релизная ветка, тег, SHA checkout и provenance согласуются до публикации.
Произвольная ветка и несовместимая с maintenance версия отвергаются.
