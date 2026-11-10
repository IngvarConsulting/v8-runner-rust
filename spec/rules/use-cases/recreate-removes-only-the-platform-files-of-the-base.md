---
id: INV.USE-CASES.RECREATE-REMOVES-ONLY-THE-PLATFORM-FILES-OF-THE-BASE
check:
  - tests/cli_infobase_recreate.rs::a_recreated_file_base_is_new_and_keeps_a_foreign_file
  - tests/cli_infobase_recreate.rs::a_recreate_preview_names_what_it_would_remove_and_touches_nothing
  - src/platform/file_infobase.rs::only_top_level_1cv8_names_are_platform_entries
---

# Пересоздание файловой базы убирает только записи платформы

`infobase create --recreate` у файловой базы убирает из её каталога только записи верхнего
уровня, чьи имена начинаются на `1Cv8` (на Windows без учёта регистра): файл базы, журнал
регистрации `1Cv8Log`, полнотекстовый индекс `1Cv8FTxt`, файлы-спутники и блокировку начала
сеансов `1Cv8.cdn`. Запись берётся целиком, ссылки не разворачиваются, вложенные каталоги не
обходятся. Посторонние файлы каталога остаются. Замок раннера и метка владельца лежат рядом с
каталогом и тоже остаются.

Записи откладываются в каталог `.<имя каталога>.v8-runner.recreate` рядом с каталогом базы, на
том же томе, и удаляются, только когда новая база создана. Ответ называет каждую запись в
`removed`; превью называет их и ничего не пишет.

Без базы `--recreate` просто создаёт её, и `removed` пуст. Без ключа существующая база — отказ,
который советует `--recreate` (`INV.CLI.INFOBASE-CREATE-FOLLOWS-THE-TARGET-KIND`).

Источник: замер [#597](../../../references/1c/confirmed-runtime-measurements.md), раздел
«Пересоздание базы»: все записи, которые заводит платформа, начинаются на `1Cv8`, а создание
поверх одних спутников проходит, но оставляет в новой базе старые журнал и индекс.
