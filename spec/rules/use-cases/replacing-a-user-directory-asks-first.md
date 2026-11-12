---
id: INV.USE-CASES.REPLACING-A-USER-DIRECTORY-ASKS-FIRST
check:
  - src/platform/git.rs::an_ignored_file_is_at_risk_although_the_tree_looks_clean
  - tests/cli_dump.rs::a_dump_refuses_to_destroy_work_version_control_cannot_give_back
  - src/platform/git.rs::a_regenerated_file_at_the_root_is_not_at_risk
  - src/use_cases/destruction_guard.rs::an_ignored_version_file_at_the_root_is_not_a_loss
  - src/use_cases/destruction_guard.rs::a_replacement_that_does_not_regenerate_the_version_file_protects_it
  - tests/cli_dump.rs::a_full_dump_replaces_an_ignored_version_file_without_asking
  - tests/cli_pull_memory.rs::a_full_dump_over_the_directory_asks_the_replacement_guard
  - tests/cli_dump.rs::an_incremental_pull_refuses_over_work_version_control_cannot_give_back
  - tests/cli_dump.rs::a_partial_pull_refuses_over_an_uncommitted_edit
  - tests/cli_live_agent.rs::a_pull_the_guard_refuses_answers_alike_and_sends_the_platform_no_work
---

# Замена каталога человека спрашивает заранее

Перед тем как заменить каталог, который назвал человек, или переписать его выгрузкой поверх
каталога, раннер спрашивает систему контроля версий, что в нём не восстановить. Выгрузка
`pull` спрашивает до первой рабочей команды платформе при любом режиме: по изменившемуся, выборкой
объектов, полной поверх каталога без годного файла версий, заменой каталога или проекта EDT;
замена спрашивает ещё раз перед самой публикацией. Живой агент к этому времени может уже
подниматься и подключаться к базе (`INV.PLATFORM.A-LIVE-AGENT-STARTS-WHILE-THE-COMMAND-PREPARES`); отказ сторожа отвечает тем же, что и без него, а
поднятый агент живёт по обычным правилам живого агента. Отказ называет действие: `refusing to
replace` или `refusing to overwrite`. Кроме ответов «терять нечего» и «есть безвозвратное»,
бывает «ответа нет»; что он значит, держит
`INV.USE-CASES.AN-UNTRACKED-DIRECTORY-IS-REFUSED-NOT-REPLACED`, а как читается ответ гита —
`INV.USE-CASES.A-GIT-WARNING-DOES-NOT-CANCEL-ITS-ANSWER`.

Безвозвратно — то, что живёт только на диске: файл вне учёта, файл в игноре, правка
поверх индекса, разметка незавершённого слияния. Проиндексированное сюда не входит. Не входит и
`ConfigDumpInfo.xml` в корне каталога, который заменяет или переписывает выгрузка в формате Конфигуратора:
выгрузка поверх каталога его не трогает, а замена переносит в новое дерево как был
([правило](the-version-file-lives-only-in-the-runner-memory.md)). Одноимённый файл глубже корня, как
и опись в каталоге, который заменяет преобразование или проект EDT, остаётся под защитой.

Каталог, который раннер завёл сам, сторож не спрашивает —
`INV.USE-CASES.A-RUNNER-DIRECTORY-IS-NOT-QUESTIONED`.
