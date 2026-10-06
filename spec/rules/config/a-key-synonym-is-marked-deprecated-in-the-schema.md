---
id: INV.CONFIG.A-KEY-SYNONYM-IS-MARKED-DEPRECATED-IN-THE-SCHEMA
check:
  - tests/config_schema_synonyms.rs::every_key_synonym_of_the_model_is_deprecated_in_the_schema
  - src/config/schema.rs::the_infobase_synonym_is_deprecated_and_the_map_keys_are_identifiers
---

# Прежний ключ конфигурации помечен в схеме устаревшим

Каждый ключ, который модель принимает как синоним, присутствует в опубликованной схеме с
`deprecated: true`; ключ, принимаемый моделью и отсутствующий в схеме, — нарушение.
