---
id: INV.USE-CASES.A-MISSING-VERSION-FILE-IS-KNOWN-BEFORE-THE-PLATFORM-STARTS
check:
  - src/use_cases/dump_config.rs::an_incremental_dump_without_a_version_file_runs_full_before_the_platform_starts
  - src/use_cases/dump_config.rs::ibcmd_dump_incremental_uses_sync_against_resolved_target
  - src/use_cases/dump_config.rs::dump_incremental_edt_without_a_version_file_in_the_snapshot_is_full
  - src/use_cases/dump_config.rs::a_foreign_format_version_turns_the_dump_full
  - src/platform/dump_format.rs::the_version_is_read_from_the_root_attribute_only
---

# Отсутствие файла версий известно до запуска платформы

Выгрузка по изменившемуся при отсутствующем у раннера файле версий или при чужой версии
его формата переводится в полную до запуска платформы: `-update` в аргументах не
появляется, а ответ называет причину. Исключение — восстановление файла версий по
`INV.USE-CASES.A-VERSION-FILE-ALONE-IS-DUMPED-ONLY-WHEN-THE-DIRECTORY-MATCHES-THE-BASE`.

Полная выгрузка вместо выгрузки по изменившемуся ложится поверх каталога набора и лишнего
в нём не удаляет (`INV.CLI.PULL-LAYS-THE-DUMP-OVER-THE-DIRECTORY`); снимок формата EDT
заменяется целиком. Версия формата читается из атрибута `version` корня файла версий;
нераспознанная версия чужая. Версию, которую пишет выбранная платформа, раннер берёт из
своей таблицы, подтверждённой документацией или замером; для платформы вне таблицы
прочитанную версию он чужой не считает.
