---
id: INV.USE-CASES.CHANGES-ARE-DETECTED-ON-DEMAND
check:
  - src/use_cases/build_project.rs::changed_extension_only_loads_extension_and_preserves_other_storage
  - src/use_cases/build_project.rs::source_set_build_analyzes_and_loads_only_requested_source_set
  - src/change_detection/source_sets.rs::analysis_state_lies_under_the_work_path_by_logical_context
  - src/change_detection/scanner.rs::a_known_file_is_hashed_only_when_touched_within_the_margin
  - src/change_detection/analyzer.rs::a_file_rewritten_with_the_same_content_is_not_a_change
  - src/change_detection/source_sets.rs::ad_hoc_analysis_never_reads_or_writes_memory_and_empty_sources_skip
  - src/change_detection/scanner.rs::service_and_generated_paths_are_never_scanned
  - src/change_detection/source_sets.rs::a_source_set_rooted_at_a_service_named_directory_is_analyzed
  - src/change_detection/source_sets.rs::a_generated_designer_copy_named_build_is_analyzed
  - tests/architecture_guardrails.rs::change_detection_has_no_background_watcher
  - tests/architecture_guardrails.rs::change_detection_never_reads_the_executor_choice
---

# Изменения ищет та команда, которой нужен ответ

Фонового наблюдателя нет: анализ изменений запускает та команда, которой нужно решение о
сборке, экспорте или загрузке. Состояние анализа лежит под `workPath` по логическому
контексту набора.

Отсутствующий снимок не объявляется ошибкой хранилища: первая сборка видит существующие
файлы как добавленные, пустой набор пропускается.

Кандидаты отбираются по времени изменения с запасом и подтверждаются хешем. Служебные и
порождённые каталоги пропускаются только внутри набора: корень набора, в том числе
порождённая копия набора EDT в `workPath/designer`, сканируется всегда, даже если назван
как служебный каталог. Анализ отвечает только на вопрос, что делать, — выбора исполнителя
он не касается.
