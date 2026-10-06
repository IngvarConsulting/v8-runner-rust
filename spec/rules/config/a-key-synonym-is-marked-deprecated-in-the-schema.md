---
id: INV.CONFIG.A-KEY-SYNONYM-IS-MARKED-DEPRECATED-IN-THE-SCHEMA
check:
  - tests/config_schema_synonyms.rs::every_key_synonym_of_the_model_is_deprecated_in_the_schema
  - src/config/schema.rs::the_infobase_synonym_is_deprecated_and_the_map_keys_are_identifiers
  - src/config/schema.rs::every_root_synonym_the_loader_folds_is_deprecated_in_the_schema
---

# Прежний ключ конфигурации помечен в схеме устаревшим

Каждый ключ, который модель или загрузчик принимает как синоним, присутствует в
опубликованной схеме с `deprecated: true`; ключ, принимаемый моделью или загрузчиком и
отсутствующий в схеме, — нарушение.
