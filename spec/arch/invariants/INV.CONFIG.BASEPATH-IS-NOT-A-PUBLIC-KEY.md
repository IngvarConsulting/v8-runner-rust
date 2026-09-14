---
id: INV.CONFIG.BASEPATH-IS-NOT-A-PUBLIC-KEY
status: active
governs: product
decision: DEC.2026-04-20.V8PROJECT-YAML-IS-THE-PROJECT-CONTRACT
check: src/config/schema.rs::main_schema_and_loader_reject_unknown_keys
scope: [config]
---

# Базовый путь проекта не задаётся ключом

Внутренний базовый путь равен каталогу основного конфига; ключа для него в публичном контракте нет.
