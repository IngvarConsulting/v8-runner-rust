---
id: INV.USE-CASES.NO-TRANSPORT-TYPES-IN-THE-USE-CASE-LAYER
status: active
governs: process
decision: DEC.2026-04-20.USE-CASES-STAY-TRANSPORT-NEUTRAL
check: tests/use_case_boundaries.rs::use_cases_do_not_depend_on_transport_or_presentation_types
scope: [use-cases]
---

# В слое сценариев нет типов транспорта

Слой сценариев не ссылается на разбор аргументов, представление и типы MCP — ни в сигнатурах, ни внутри.
