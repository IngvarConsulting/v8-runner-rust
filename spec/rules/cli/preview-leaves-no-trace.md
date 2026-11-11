---
id: INV.CLI.PREVIEW-LEAVES-NO-TRACE
check:
  - tests/contract_previews.rs::no_leaf_with_a_preview_creates_the_work_path
  - tests/contract_previews.rs::a_named_action_log_path_is_not_honoured_by_a_preview
  - tests/contract_previews.rs::no_preview_changes_what_a_real_build_left_in_the_work_path
  - src/use_cases/source_changes.rs::a_preview_leaves_the_git_index_untouched
---

# Превью не оставляет следов в файловой системе

Превью не создаёт ничего: ни целей, ни артефактов, ни промежуточных каталогов, ни
рабочего каталога, ни файла журнала действий. Названный через `V8TR_ACTION_LOG_FILE` путь
под превью тоже не исполняется. Вопрос к git о наборе превью задаёт с `GIT_OPTIONAL_LOCKS=0`:
`.git/index` не меняется, хотя без превью тот же вопрос обновляет в нём сведения о файлах. Проверяется по отсутствию файлов, а не по отсутствию
вызова: запись о вызове несёт конверт на stdout.
