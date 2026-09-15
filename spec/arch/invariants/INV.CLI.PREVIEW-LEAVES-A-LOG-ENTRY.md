---
id: INV.CLI.PREVIEW-LEAVES-A-LOG-ENTRY
status: active
governs: product
decision: DEC.2026-09-11.PREVIEW-STOPS-BEFORE-THE-PROVIDER-IS-DISPATCHED
check: tests/contract_previews.rs::every_preview_leaves_a_line_in_the_action_log
scope: [cli]
---

# Превью оставляет запись в журнале действий

Строка о вызове появляется в журнале и для превью: оно не прячется, хотя предмет не
меняет.

Замер 14.09.2026 нашёл, что `dump --dry-run` оставлял журнал пустым; фальсификатор,
написанный 15.09, нашёл то же у `init`, `load` и `make`. Все четыре превью теперь пишут
свою строку, и проверка держит шесть команд.
