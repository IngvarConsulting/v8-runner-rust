---
id: INV.USE-CASES.A-GONE-OWNER-IS-REPLACED-AND-NAMED
check:
  - tests/cli_infobase_owner.rs::a_gone_owner_is_replaced_and_named
  - tests/cli_infobase_owner.rs::a_project_copied_whole_leaves_no_live_owner
  - tests/cli_infobase_owner.rs::an_unreadable_local_layer_of_the_owner_keeps_it_alive
  - tests/cli_infobase_owner.rs::an_owner_on_another_machine_is_never_replaced
  - src/use_cases/infobase_owner.rs::a_host_rename_keeps_the_machine
---

# Ушедшего владельца сменяют и называют

На базе, названной в местном слое, владельца, чей каталог на этой машине исчез или больше не
объявляет базу, команда записи сменяет сама, и ответ об этом говорит. Живого владельца
раннер не трогает: копию с другой машины, как и копию этой машины, чей местный слой нельзя
прочитать, раннер считает живой и несогласной. Машину называет её идентификатор, который
переживает смену имени хоста, а объявленный путь базы сравнивается канонически от каталога
владельца — поэтому проект, скопированный целиком вместе с меткой, прежнего владельца живым не
оставляет. Команды записи и чтения определены в
`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
