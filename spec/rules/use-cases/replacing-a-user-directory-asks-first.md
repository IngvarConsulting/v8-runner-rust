---
id: INV.USE-CASES.REPLACING-A-USER-DIRECTORY-ASKS-FIRST
check:
  - src/platform/git.rs::an_ignored_file_is_at_risk_although_the_tree_looks_clean
  - src/platform/git.rs::an_unreadable_subdirectory_is_unknown_not_clean
  - tests/cli_dump.rs::a_dump_refuses_to_destroy_work_version_control_cannot_give_back
  - src/platform/git.rs::a_regenerated_file_at_the_root_is_not_at_risk
  - src/use_cases/destruction_guard.rs::an_ignored_version_file_at_the_root_is_not_a_loss
  - src/use_cases/destruction_guard.rs::a_replacement_that_does_not_regenerate_the_version_file_protects_it
  - tests/cli_dump.rs::a_full_dump_replaces_an_ignored_version_file_without_asking
  - tests/cli_pull_memory.rs::a_full_dump_over_the_directory_asks_the_replacement_guard
  - tests/cli_dump.rs::an_incremental_pull_refuses_over_work_version_control_cannot_give_back
  - tests/cli_dump.rs::a_partial_pull_refuses_over_an_uncommitted_edit
  - src/platform/git.rs::a_warning_on_a_successful_status_keeps_the_answer
  - tests/cli_convert.rs::convert_without_source_set_processes_all_source_sets_into_work_path_out
---

# Замена каталога человека спрашивает заранее

Перед тем как заменить каталог, который назвал человек, или переписать его выгрузкой поверх
каталога, раннер спрашивает систему контроля версий, что в нём не восстановить. Выгрузка
`pull` спрашивает до запуска платформы при любом режиме: по изменившемуся, выборкой
объектов, полной поверх каталога без годного файла версий, заменой каталога или проекта EDT;
замена спрашивает ещё раз перед самой публикацией. Отказ называет действие: `refusing to
replace` или `refusing to overwrite`. Ответов три: терять нечего, есть безвозвратное,
ответа нет; что значит третий, держит `INV.USE-CASES.AN-UNTRACKED-DIRECTORY-IS-REFUSED-NOT-REPLACED`. Удачный
ответ гита остаётся ответом и с предупреждением в stderr. Неполный перечень — это «ответа
нет»: подкаталог, который не прочесть, раннер находит обходом каталога, а не по тексту гита.

Безвозвратно — то, что живёт только на диске: файл вне учёта, файл в игноре, правка
поверх индекса, разметка незавершённого слияния. Проиндексированное сюда не входит. Не входит и
`ConfigDumpInfo.xml` в корне каталога, который заменяет или переписывает выгрузка в формате Конфигуратора:
она пишет его заново, а штатно он лежит в игноре (#162). Одноимённый файл глубже корня, как
и опись в каталоге, который заменяет преобразование или проект EDT, остаётся под защитой.

Каталог, который раннер завёл сам, сторож не спрашивает: служебный снимок Конфигуратора у
проекта EDT и вывод `convert` по умолчанию под `workPath`. Каталог, названный `convert
--output`, принадлежит человеку.
