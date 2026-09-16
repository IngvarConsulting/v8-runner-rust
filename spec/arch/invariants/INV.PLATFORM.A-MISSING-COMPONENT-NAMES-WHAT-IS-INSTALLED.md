---
id: INV.PLATFORM.A-MISSING-COMPONENT-NAMES-WHAT-IS-INSTALLED
status: active
governs: product
decision: DEC.2026-09-16.A-MISSING-COMPONENT-IS-NAMED-WITH-ITS-INSTALLATIONS
check: src/platform/locator.rs::a_missing_component_is_named_together_with_the_installations_that_have_it
scope: [platform]
---

# Ненайденная утилита платформы названа вместе с описью установок

Отказ по утилите платформы перечисляет найденные установки, называет недостающий
компонент и версии, в которых он есть. Одно имя файла отказом не считается.
