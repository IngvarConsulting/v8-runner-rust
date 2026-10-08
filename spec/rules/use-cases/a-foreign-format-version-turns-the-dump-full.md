---
id: INV.USE-CASES.A-FOREIGN-FORMAT-VERSION-TURNS-THE-DUMP-FULL
check:
  - src/use_cases/dump_config.rs::an_unrecognized_format_turns_the_dump_full_and_a_version_is_foreign_only_by_measurement
  - src/platform/dump_format.rs::the_table_holds_the_measured_platforms
  - tests/cli_dump.rs::a_foreign_format_version_turns_the_pull_full_before_the_platform_starts
---

# Чужая версия формата делает выгрузку полной

Версия формата из атрибута `version` корня файла версий сверяется до запуска платформы с
той, что пишет выбранная платформа. Не та — выгрузка по изменившемуся переводится в полную
поверх каталога так же, как без файла версий
(`INV.USE-CASES.A-MISSING-VERSION-FILE-IS-KNOWN-BEFORE-THE-PLATFORM-STARTS`), и ответ
называет прочитанную версию и ту, что пишет платформа. Версию, которую пишет платформа, раннер берёт из таблицы
`src/platform/dump_format.rs`, куда попадают только замеренные строки; для платформы вне
таблицы прочитанную версию он чужой не считает.

Источник: [`problems.html#p4`](../../../docs/site/problems.html#p4).
