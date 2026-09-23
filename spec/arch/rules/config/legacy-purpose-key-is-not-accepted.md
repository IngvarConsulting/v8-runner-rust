---
id: INV.CONFIG.LEGACY-PURPOSE-KEY-IS-NOT-ACCEPTED
check: [src/config/loader.rs::load_config_rejects_legacy_source_set_purpose_key]
---

# Прежний ключ назначения набора не принимается

Тип набора задаётся ключом типа; прежнее имя ключа отклоняется, а не читается молча.
