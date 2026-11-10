---
id: INV.CLI.THE-LOCAL-LOCK-FOLLOWS-THE-WORKPATH-LOCK
check:
  - src/use_cases/transport.rs::a_stand_command_takes_the_local_lock_after_the_work_path_lock
---

# Замок `localPath` берётся после замка `workPath`

Команда, меняющая стенд, — `start`, `stop` и `restart` у `dbms`, `cluster` и `standalone`, а
также `infobase create` базы управляемого стенда — берёт замок `<localPath>/.lock` после замка
своего `workPath`. Занятый замок — отказ `workspace_busy`: запуски одного проекта с разными
`workPath` (`--workdir`) не меняют один стенд разом. Стенд другого проекта в общем `localPath`
замок не охраняет — его записи не трогает сама команда стенда
(`INV.PLATFORM.A-SERVICE-IS-TOLD-APART-BY-ITS-IDENTITY`). Занятый `workPath` отказывает раньше, и `localPath`
тогда не заводится. `status` и команды, стенда не меняющие, этого замка не берут.
