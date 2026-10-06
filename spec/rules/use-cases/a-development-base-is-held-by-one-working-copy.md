---
id: INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY
check:
  - tests/cli_infobase_owner.rs::a_write_on_a_base_of_another_copy_is_refused_and_names_the_owner
  - tests/cli_infobase_owner.rs::an_owner_on_another_machine_is_never_replaced
  - src/use_cases/transport.rs::a_base_of_another_copy_stops_the_dispatch_before_the_scenario
  - tests/architecture_guardrails.rs::the_owner_of_a_file_base_is_checked_in_one_place
  - tests/cli_infobase_owner.rs::a_held_refusal_on_a_shared_base_leads_to_an_own_base
---

# Базу для разработки держит одна рабочая копия

Команда записи на базе — та, что создаёт, меняет или заменяет эту базу, выгружает её в новый
проект (`clone --from`), открывает на ней клиент или Конфигуратор для работы либо сверяет
ответ со своими исходниками и памятью. Пакетный запуск платформы для выгрузки, снимка или
отчёта, как и `status`, командой записи её не делает. Команда чтения — та, что открывает
базу, но командой записи на ней не является.

База другой рабочей копии — файловая база, записанная в метке за другой живой копией: её
каталог на этой машине есть и объявляет базу, либо она с другой машины. На ней команда
записи отказывает, если делить базу не согласны эта копия или кто-то из владельцев
(`INV.USE-CASES.A-BASE-IS-SHARED-BY-CONSENT-OF-EVERY-COPY`), и называет копию-владельца,
как освободить базу и где лежит метка.

Отказ на базе другой рабочей копии отвечает кодом `infobase_held` (`CTR.WIRE.COMMAND-ENVELOPE`):
повтор его не снимает. Следующий шаг отказа — первый и безопасный выход, своя чистая база
(`infobase create`); копию базы (`infobase create --from`) и общую базу (`shared: true`) отказ
называет текстом. Проверка владельца идёт на границе команды, за замком базы, — одна для
командной строки и MCP.

В словаре сайта команды записи — `push`, `pull`, `apply`, `reset`, `upload`,
`infobase restore`, `infobase create`, `extensions set`, `test`, `check` для исходников XML,
`launch` и `diff`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
