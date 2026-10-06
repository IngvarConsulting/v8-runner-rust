---
id: INV.CLI.PULL-ALL-PULLS-EACH-PROJECT-SET-AS-PULL-SET
check:
  - tests/cli_pull_all.rs::an_existing_set_is_left_as_declared
  - tests/cli_pull_all.rs::an_extension_without_a_set_is_declared_and_pulled
---

# `pull --all` выгружает наборы проекта как `pull <SET>`

Набор конфигурации и набор расширения, которое в базе есть, `pull --all` выгружает тем же
сценарием, что `pull <SET>`: по изменившемуся, со сторожем каталога и памятью базы. Запись
набора в `v8project.yaml` не меняется, даже когда его каталог лежит не по соглашению
`src/ext/<Name>`.
