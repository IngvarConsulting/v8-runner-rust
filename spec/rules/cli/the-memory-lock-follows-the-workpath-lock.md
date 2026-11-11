---
id: INV.CLI.THE-MEMORY-LOCK-FOLLOWS-THE-WORKPATH-LOCK
check:
  - src/use_cases/transport.rs::a_write_command_takes_the_memory_lock_after_the_work_path_lock
---

# Замок памяти базы берётся после замка `workPath`

Память о базе лежит под `localPath` и общая для запусков с разными `workPath` (`--workdir`,
проекция с абсолютными путями). Команда, которая пишет базу, берёт замок
`<localPath>/infobases/<база>/.lock` после замка `workPath` и замка `localPath`, если берёт
его, и до замка файловой базы. Занятый замок — отказ `workspace_busy`. Занятый `workPath`
отказывает раньше, и память тогда не заводится. Команды чтения базы (`status --deep`,
`diff --against`) и команды, базы не открывающие, этого замка не берут. У базы, адрес которой
не распознан, памяти нет, и замка тоже.
