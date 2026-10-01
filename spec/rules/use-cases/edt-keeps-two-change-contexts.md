---
id: INV.USE-CASES.EDT-KEEPS-TWO-CHANGE-CONTEXTS
check:
  - tests/cli_build.rs::build_edt_text_interleaves_export_stage_after_edt_log
  - src/change_detection/source_sets.rs::edt_designer_contexts_use_nested_designer_directory
  - tests/architecture_guardrails.rs::change_detection_never_reads_the_executor_choice
---

# У формата EDT две ступени состояния изменений

Обнаружение изменений идёт по двум контекстам на набор: `edt-<набор>` решает, нужен ли
экспорт проекта в формат Конфигуратора, `designer-<набор>` — нужна ли частичная или полная
загрузка порождённых файлов. Для формата Конфигуратора контекст один.

Ступени говорят только о том, нужен ли шаг; чем грузить, решает выбор исполнителя.
