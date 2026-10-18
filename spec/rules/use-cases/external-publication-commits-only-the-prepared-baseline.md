---
id: INV.USE-CASES.EXTERNAL-PUBLICATION-COMMITS-ONLY-THE-PREPARED-BASELINE
check:
  - tests/cli_external_full_pull.rs::rejecting_the_selected_executor_starts_no_platform_and_preserves_baseline
  - tests/cli_external_full_pull.rs::rejecting_the_ready_tree_keeps_every_baseline_and_the_existing_sources
  - tests/cli_external_full_pull.rs::accepted_external_full_pull_remembers_the_original_source_for_the_next_push
  - tests/cli_external_full_pull.rs::eof_after_the_callers_publication_keeps_its_fact_and_does_not_fabricate_memory
  - tests/cli_external_full_pull.rs::an_edit_after_publication_is_not_absorbed_into_the_prepared_baseline
  - tests/cli_external_full_pull.rs::the_workspace_lock_is_held_while_waiting_for_both_caller_decisions
  - tests/cli_external_full_pull.rs::a_memory_write_failure_keeps_the_published_sources_and_names_the_unsaved_baseline
---

# Внешняя публикация подтверждает подготовленную память итогового набора

При внешней публикации полной XML-выгрузки вызывающий владеет временным
деревом и заменой цели; раннер владеет памятью синхронизации. Выгрузка сохраняет
тождество итогового набора и базы. Хеши, поколение и копия файла версий
подготавливаются до передачи дерева вызывающему и фиксируются после
подтверждения состоявшейся публикации. До подтверждения прежняя память
сохраняется, в том числе при отказе, отмене или неизвестном исходе публикации.

После подтверждения локальное завершение памяти доводится до результата
с удержанием блокировок; поздняя отмена не выдаётся за несостоявшуюся
публикацию. Оно использует подготовленные данные: последующие правки
опубликованных файлов остаются изменениями. Ошибка подготовки или записи
памяти сохраняет правдивый исход публикации и называет несохранённую память.
