---
id: DEC.2026-09-14.BUILDER-KEY-IS-REMOVED
status: active
governs: product
realized: [tests/provider_matrix.rs::a_config_with_the_builder_key_is_refused_with_the_replacement_named, tests/architecture_guardrails.rs::the_shipped_skill_never_names_the_removed_builder_key]
supersedes: []
superseded-by: null
establishes: [INV.CONFIG.BUILDER-KEY-IS-REJECTED, CTR.CONFIG.V8PROJECT-SCHEMA, CTR.WIRE.CONFIG-INIT-DATA]
changes: [CTR.CONFIG.V8PROJECT-SCHEMA, CTR.WIRE.CONFIG-INIT-DATA]
---

# Ключ `builder` снимается

**Решение.** Ключ `builder` удаляется из конфигурационного контракта: не
переименовывается и не остаётся синонимом. Ключ `format` остаётся — он описывает
формат исходников и к выбору исполнителя отношения не имеет.

**Почему.** Ключ обещал глобальный выбор исполнителя, а выбор пооперационный.
Оставить его синонимом значило бы сохранить обещание, которого система не даёт, и
заставить читателя гадать, какие команды его слушают.

**Цена.** Снятие затрагивает около тридцати файлов и ломает существующие
`v8project.yaml`: конфиг с `builder` перестаёт проходить валидацию и требует
правки одной строки.
