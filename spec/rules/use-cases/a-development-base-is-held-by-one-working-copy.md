---
id: INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY
check:
  - tests/cli_infobase_owner.rs::a_write_on_a_base_of_another_copy_runs_with_a_warning_and_names_the_owner
  - tests/cli_infobase_owner.rs::an_owner_on_another_machine_is_never_replaced
  - src/use_cases/infobase_owner.rs::a_host_rename_keeps_the_machine
  - tests/architecture_guardrails.rs::the_owner_of_a_file_base_is_checked_in_one_place
---

# Базу для разработки держит одна рабочая копия

Команда записи на базе — та, что создаёт, меняет или заменяет эту базу, выгружает её в новый
проект (`clone --from`), открывает на ней клиент или Конфигуратор для работы либо сверяет
ответ со своими исходниками и памятью. Пакетный запуск платформы для выгрузки, снимка или
отчёта, как и `status`, командой записи её не делает. Команда чтения — та, что открывает
базу, но командой записи на ней не является.

Файловую базу держит рабочая копия, записанная в её метке
(`CTR.USE-CASES.INFOBASE-OWNER-MARKER`). База другой рабочей копии — записанная в метке за
другой живой копией: её каталог на этой машине есть и объявляет базу, либо она с другой
машины. Команда записи другой копии владельцем не делает и метку не меняет: владельцем
остаётся прежняя копия, а как такая команда идёт, говорит
`INV.USE-CASES.A-WRITE-ON-A-BASE-OF-ANOTHER-COPY-RUNS-WITH-A-WARNING`. Проверка владельца
идёт на границе команды, за замком базы, — одна для командной строки и MCP.

В словаре сайта команды записи — `push`, `pull`, `apply`, `reset`, `upload`,
`infobase restore`, `infobase create`, `extensions set`, `test`, `check` для исходников XML,
`launch` и `diff`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
