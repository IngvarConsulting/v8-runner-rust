---
id: CTR.CONFIG.V8PROJECT-SCHEMA
status: active
governs: product
version: 1
decision: DEC.2026-04-20.V8PROJECT-YAML-IS-THE-PROJECT-CONTRACT
artifact: docs/schemas/v8project.schema.json
producer: src/config/schema.rs
consumers: [docs, cli]
check: src/config/schema.rs::generated_schema_artifacts_are_current
scope: [config, docs]
---

# Опубликованные схемы конфигурации

Форма `v8project.yaml` и его локального слоя опубликована двумя JSON-схемами. Схемы порождаются из типизированной модели и обязаны совпадать с ней: расхождение
валит проверку, а не обнаруживается в редакторе пользователя. Артефакт обновляется
командой `UPDATE_CONFIG_SCHEMAS=1 cargo test generated_schema_artifacts_are_current`.
