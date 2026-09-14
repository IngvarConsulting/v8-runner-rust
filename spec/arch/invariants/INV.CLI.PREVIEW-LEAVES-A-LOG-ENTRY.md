---
id: INV.CLI.PREVIEW-LEAVES-A-LOG-ENTRY
status: planned
governs: product
decision: DEC.2026-09-11.PREVIEW-STOPS-BEFORE-THE-PROVIDER-IS-DISPATCHED
check: null
scope: [cli]
---

# Превью оставляет запись в журнале действий

Строка о вызове появляется в журнале и для превью: оно не прячется, хотя предмет не
меняет.

Замер 14.09.2026: правило сейчас не держится. У `dump --dry-run` файл журнала
действий создаётся пустым, поэтому тест написан не был — сначала нужна правка
поведения, потом фальсификатор.
