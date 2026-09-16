---
id: INV.DOCS.A-RECORD-NAME-SURVIVES-A-WINDOWS-CHECKOUT
status: active
governs: process
decision: DEC.2026-09-16.A-RECORD-NAME-SURVIVES-A-WINDOWS-CHECKOUT
check: [tests/arch_registry.rs::a_record_name_survives_a_windows_checkout, tests/arch_registry.rs::registry_records_match_the_published_schema_and_the_index_is_current]
scope: [docs]
---

# Базовое имя файла записи не является именем DOS-устройства

Часть имени файла до первой точки не совпадает с CON, PRN, AUX, NUL, COM1…COM9,
LPT1…LPT9: такое имя Windows отказывается создавать с любым расширением, и дерево
перестаёт выкладываться целиком.
