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
---

# Замена каталога человека спрашивает заранее

Перед тем как заменить каталог, который назвал человек, раннер спрашивает систему
контроля версий, что в нём не восстановить. Ответов три: терять нечего, есть
безвозвратное, ответа нет.

Безвозвратно — то, что живёт только на диске: файл вне учёта, файл в игноре, правка
поверх индекса, разметка незавершённого слияния. Проиндексированное сюда не входит. Не входит и
`ConfigDumpInfo.xml` в корне каталога, который заменяет выгрузка в формате Конфигуратора:
она пишет его заново, а штатно он лежит в игноре (#162). Одноимённый файл глубже корня, как
и опись в каталоге, который заменяет преобразование или проект EDT, остаётся под защитой.

Незнание не приравнивается к угрозе: работа идёт, как шла до сторожа, и защиты в
этом случае не обещают.
