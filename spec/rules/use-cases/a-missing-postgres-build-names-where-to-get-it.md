---
id: INV.USE-CASES.A-MISSING-POSTGRES-BUILD-NAMES-WHERE-TO-GET-IT
check:
  - src/use_cases/managed_stand/mod.rs::a_start_without_the_build_names_where_to_get_it
  - src/use_cases/managed_stand/mod.rs::a_restart_without_the_build_leaves_the_running_postgres_alone
---

# Отказ без сборки PostgreSQL называет, где её взять

Раннер сборку PostgreSQL для 1С не скачивает и не устанавливает. Если `tools.postgres.path` не объявлен
или в каталоге нет `pg_ctl`, `start` и `restart` отказывают с родом `environment` и ничего не запускают;
`restart` проверяет сборку до остановки, и работающий PostgreSQL остаётся работать. Текст
отказа называет ключ `tools.postgres.path`, адрес бесплатной сборки «PostgreSQL for 1C»
(текущий установщик в каталоге `https://repo.postgrespro.ru/1c/1c-17/win/`), её лицензию (PostgreSQL License, `license.txt` в
сборке) и то, что установщику нужны права администратора. Скачать, установить или спросить человека
решает вызывающий.
