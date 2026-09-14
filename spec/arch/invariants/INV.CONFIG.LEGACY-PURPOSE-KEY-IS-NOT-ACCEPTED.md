---
id: INV.CONFIG.LEGACY-PURPOSE-KEY-IS-NOT-ACCEPTED
status: active
governs: product
decision: DEC.2026-04-20.SOURCE-SET-IS-THE-UNIT-OF-ORCHESTRATION
check: src/config/loader.rs::load_config_rejects_legacy_source_set_purpose_key
scope: [config]
---

# Прежний ключ назначения набора не принимается

Тип набора задаётся ключом типа; прежнее имя ключа отклоняется, а не читается молча.
