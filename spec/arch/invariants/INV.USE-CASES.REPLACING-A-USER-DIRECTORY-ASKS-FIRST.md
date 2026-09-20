---
id: INV.USE-CASES.REPLACING-A-USER-DIRECTORY-ASKS-FIRST
status: active
governs: product
decision: DEC.2026-09-20.A-DUMP-NEVER-LOSES-UNRECOVERABLE-WORK
check: [src/platform/git.rs::an_ignored_file_is_at_risk_although_the_tree_looks_clean, src/platform/git.rs::an_unreadable_subdirectory_is_unknown_not_clean, tests/cli_dump.rs::a_dump_refuses_to_destroy_work_version_control_cannot_give_back]
scope: [use-cases]
---

# Замена каталога человека спрашивает заранее

Перед тем как заменить каталог, который назвал человек, раннер спрашивает систему
контроля версий, что в нём не восстановить. Ответов три: терять нечего, есть
безвозвратное, ответа нет.

Безвозвратно — то, что живёт только на диске: файл вне учёта, файл в игноре, правка
поверх индекса, разметка незавершённого слияния. Проиндексированное сюда не входит.

Незнание не приравнивается к угрозе: работа идёт, как шла до сторожа, и защиты в
этом случае не обещают.
