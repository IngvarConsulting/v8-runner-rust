---
id: INV.CLI.A-FILE-BASE-OF-AN-EDT-PROJECT-IS-ASSEMBLED-FROM-ITS-SOURCES
check:
  - tests/cli_init.rs::a_file_base_of_an_edt_project_is_assembled_by_ibcmd_from_its_sources
  - tests/cli_init.rs::init_edt_imports_projects_in_configuration_then_extension_order
---

# Файловая база проекта EDT собирается из его исходников

`infobase create` в проекте формата EDT сначала заводит рабочую область, затем переводит
основной набор в XML тем же переводом, что у `push` и `make`, — в тот же каталог, куда его
переводит `push`, — и создаёт файловую базу сразу с этой конфигурацией
(`INV.CLI.INFOBASE-CREATE-FOLLOWS-THE-TARGET-KIND`). Память знает собранный набор и его
исходники EDT: первая отправка его не переводит и не загружает.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
