---
id: INV.CLI.PULL-ALL-DECLARES-A-SET-FOR-EACH-INSTALLED-EXTENSION
check:
  - tests/cli_pull_all.rs::an_extension_without_a_set_is_declared_and_pulled
  - tests/cli_pull_all.rs::an_existing_set_is_left_as_declared
  - tests/cli_pull_all.rs::ibcmd_lists_the_installed_extensions
  - tests/cli_pull_all.rs::an_extension_named_like_another_set_is_refused_before_any_dump
  - src/use_cases/dump_config/all.rs::the_walk_keeps_project_sets_and_declares_the_rest
  - src/use_cases/dump_config/all.rs::a_taken_name_or_directory_is_refused
---

# `pull --all` объявляет набор каждому расширению базы без набора

`pull --all` спрашивает базу, какие расширения в ней установлены, вызовом выбранного
исполнителя `pull`: пакетный Конфигуратор — `/DumpDBCfgList -AllExtensions`, `ibcmd` —
`config extension list`, агент — `config extensions properties get --all-extensions`.
Ответ читается по структуре: имя на строку, поле `name`, запись JSON. Имя, которое не
является идентификатором 1С, — неверный вывод инструмента, а не набор.

Каждый набор конфигурации и расширения проекта выгружается так же, как `pull <SET>`, со
сторожем каталога и с `--force` — заменой; его запись в `v8project.yaml` не меняется. Набор
расширения, которого в базе нет, не выгружается и называется в ответе. Для расширения без
набора — имена сравниваются без учёта регистра, как их сравнивает платформа, — команда
выгружает расширение целиком в `src/ext/<Name>` от каталога проектного файла и после
удачной выгрузки дописывает набор с этим именем в `v8project.yaml`. Выключенное расширение
объявляется наравне с включённым.

Имя расширения, занятое набором другого назначения, и каталог, занятый другим набором, —
отказ до первой выгрузки; проектный файл, в который запись не дописать, — тоже.

Источник: [`cli.html#ext`](../../../docs/site/cli.html#ext).
