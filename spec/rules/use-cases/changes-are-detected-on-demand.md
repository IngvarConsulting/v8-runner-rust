---
id: INV.USE-CASES.CHANGES-ARE-DETECTED-ON-DEMAND
check:
  - src/change_detection/analyzer.rs::cancellation_during_recoverable_storage_read_is_not_fallback
  - src/use_cases/build_project.rs::cancellation_after_edt_export_keeps_completed_work_in_the_response
  - src/use_cases/build_project.rs::a_completed_full_step_records_hashes_after_deferred_cancellation
  - src/use_cases/build_project.rs::changed_extension_only_loads_extension_and_preserves_other_storage
  - src/use_cases/build_project.rs::source_set_build_analyzes_and_loads_only_requested_source_set
  - src/change_detection/source_sets.rs::analysis_state_lies_under_the_work_path_by_logical_context
  - src/change_detection/scanner.rs::every_selected_file_is_hashed_regardless_of_mtime
  - src/change_detection/scanner.rs::a_restored_copy_with_an_old_mtime_is_hashed
  - src/change_detection/scanner.rs::changed_bytes_with_the_exact_remembered_old_mtime_are_hashed
  - src/change_detection/analyzer.rs::changed_bytes_with_the_exact_remembered_old_mtime_are_a_change
  - src/change_detection/analyzer.rs::interrupted_analysis_keeps_memory_and_never_falls_back
  - src/change_detection/scanner.rs::partial_reads_fail_without_a_hash_and_cancel_before_the_next_read
  - src/change_detection/scanner.rs::an_interrupted_scan_returns_no_partial_snapshot
  - src/change_detection/scanner.rs::hashing_streams_files_larger_than_its_working_buffer
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

Изменения подтверждаются содержимым: каждый выбранный обычный файл хешируется при анализе,
включая файл с прежним размером и точно запомненным старым временем изменения. Перенос,
восстановление или новая отметка времени сами по себе не означают изменения: одинаковые
байты не вызывают загрузку. Метаданные не заменяют чтение содержимого. Хеширование идёт
потоком с ограниченным рабочим буфером без ограничения размера файла.

Анализ решения перед загрузкой замечает отмену между файлами и до и после чтения
очередного блока. Это кооперативная проверка, не гарантия срока прерывания блокирующего
чтения. Прерванный анализ не отдаёт частичный снимок, не пишет память и не превращается
в запасную полную загрузку. Отмену команды оформляет её существующий владелец
(`INV.WIRE.A-CANCELLATION-ANSWERS-AS-A-CANCELLATION`). После завершённого успешного шага или
публикации память результата записывается и при отложенной отмене.

Служебные и
порождённые каталоги пропускаются только внутри набора; корень набора сканируется всегда
(`INV.USE-CASES.A-SELECTED-ROOT-IS-SCANNED-WHATEVER-ITS-NAME`). Анализ отвечает только на вопрос, что делать, — выбора исполнителя
он не касается.
