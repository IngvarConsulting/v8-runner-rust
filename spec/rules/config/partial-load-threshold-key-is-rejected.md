---
id: INV.CONFIG.PARTIAL-LOAD-THRESHOLD-KEY-IS-REJECTED
check:
  - src/config/loader.rs::the_partial_load_threshold_key_is_refused_by_name
  - src/config/schema.rs::schemas_and_loader_reject_invalid_runtime_numeric_boundaries
  - tests/cli_config_init.rs::a_generated_config_writes_neither_a_synonym_nor_a_retired_key
  - src/use_cases/bootstrap_project.rs::a_cloned_project_carries_no_partial_load_threshold
---

# Ключ порога частичной загрузки отклоняется по имени

Ключ `push.partialLoadThreshold` — и в прежней секции `build` — не принимают ни проектный
файл, ни местный слой. Отказ называет ключ, говорит, что порога больше нет и строку нужно
удалить, а полную загрузку по желанию даёт `push --full`. `init` и `clone` этот ключ не пишут.
