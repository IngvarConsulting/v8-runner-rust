---
id: INV.USE-CASES.NO-TRANSPORT-TYPES-IN-THE-USE-CASE-LAYER
check: [tests/use_case_boundaries.rs::use_cases_do_not_depend_on_transport_or_presentation_types]
---

# В слое сценариев нет типов транспорта

Слой сценариев не ссылается на разбор аргументов, представление и типы MCP — ни в сигнатурах, ни внутри.
