---
id: INV.USE-CASES.CHANGES-ARE-DETECTED-ON-DEMAND
check:
  - src/use_cases/build_project.rs::changed_extension_only_loads_extension_and_preserves_other_storage
  - src/use_cases/build_project.rs::source_set_build_analyzes_and_loads_only_requested_source_set
  - src/change_detection/source_sets.rs::analysis_state_lies_under_the_work_path_by_logical_context
  - src/change_detection/scanner.rs::a_known_file_is_hashed_when_touched_within_the_margin_or_its_mtime_moved
  - src/change_detection/scanner.rs::a_restored_copy_with_an_old_mtime_is_hashed
  - src/change_detection/analyzer.rs::a_module_restored_with_an_old_mtime_is_a_change
  - src/change_detection/analyzer.rs::a_file_rewritten_with_the_same_content_is_not_a_change
  - src/change_detection/source_sets.rs::an_ad_hoc_base_is_remembered_by_its_address_and_empty_sources_skip
  - src/change_detection/scanner.rs::service_and_generated_paths_are_never_scanned
  - tests/architecture_guardrails.rs::change_detection_has_no_background_watcher
  - tests/architecture_guardrails.rs::change_detection_never_reads_the_executor_choice
---

# Изменения ищет та команда, которой нужен ответ

Фонового наблюдателя нет: анализ изменений запускает та команда, которой нужно решение о
сборке, экспорте или загрузке. Состояние анализа лежит под `workPath` по логическому
контексту набора.

Отсутствующий снимок не объявляется ошибкой хранилища: первая сборка видит существующие
файлы как добавленные, пустой набор пропускается.

Кандидаты отбираются по времени изменения и подтверждаются хешем: новый файл, файл, тронутый
после последней записи памяти (с запасом), и известный файл, чьё время изменения не совпадает
с запомненным в любую сторону, — так копия, восстановленная со старым временем, не теряется.
Файл с новым временем и прежним содержимым изменением не считается; его хешируют заново, пока
память не будет записана снова. Служебные и
порождённые каталоги пропускаются только внутри набора; корень набора сканируется всегда
(`INV.USE-CASES.A-SELECTED-ROOT-IS-SCANNED-WHATEVER-ITS-NAME`). Анализ отвечает только на вопрос, что делать, — выбора исполнителя
он не касается.
