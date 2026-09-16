---
id: INV.DOCS.A-RECORD-NAME-SURVIVES-A-WINDOWS-CHECKOUT
status: active
governs: process
decision: DEC.2026-09-16.A-RECORD-NAME-SURVIVES-A-WINDOWS-CHECKOUT
check: tests/arch_registry.rs::a_record_name_survives_a_windows_checkout
scope: [docs]
---

# Базовое имя файла записи не является именем DOS-устройства

Часть имени файла до первой точки не совпадает с CON, PRN, AUX, NUL, COM0…COM9,
LPT0…LPT9: такое имя Windows отказывается создавать с любым расширением, и дерево
перестаёт выкладываться целиком.
