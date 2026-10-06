---
id: INV.CLI.PULL-ALL-DECLARES-A-SET-FOR-EACH-INSTALLED-EXTENSION
check:
  - tests/cli_pull_all.rs::an_extension_without_a_set_is_declared_and_pulled
  - tests/cli_pull_all.rs::ibcmd_lists_the_installed_extensions
  - src/use_cases/dump_config/all.rs::the_walk_keeps_project_sets_and_declares_the_rest
---

# `pull --all` объявляет набор каждому расширению базы без набора

Расширение базы, у которого в проекте нет набора, `pull --all` выгружает целиком в
`src/ext/<Name>` и после удачной выгрузки дописывает в `v8project.yaml` набор с этим именем
и этим путём. Путь записан и разрешается от `basePath`, как пути остальных наборов; без
`basePath` это каталог проектного файла. Набор у расширения есть, когда какой-либо набор
расширения проекта называет его платформе этим именем; имена сравниваются без учёта
регистра, как их сравнивает платформа. Выключенное расширение объявляется наравне с
включённым.

Источник: [`cli.html#ext`](../../../docs/site/cli.html#ext).
