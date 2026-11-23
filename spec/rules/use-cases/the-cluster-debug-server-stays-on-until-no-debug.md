---
id: INV.USE-CASES.THE-CLUSTER-DEBUG-SERVER-STAYS-ON-UNTIL-NO-DEBUG
check:
  - src/use_cases/managed_stand/cluster.rs::a_debug_start_raises_the_agent_with_its_debug_server
  - src/use_cases/managed_stand/cluster.rs::the_debug_server_stays_on_until_no_debug
  - src/use_cases/managed_stand/cluster.rs::a_debug_start_on_a_cluster_running_without_it_is_refused
  - src/use_cases/managed_stand/cluster.rs::pins_written_before_the_debug_server_keep_their_registry
  - src/use_cases/managed_stand/cluster.rs::a_taken_debug_port_moves_no_registry
  - src/use_cases/managed_stand/cluster.rs::a_refused_debug_start_pins_no_debug
  - src/use_cases/managed_stand/web.rs::apache_never_takes_the_debug_port_of_the_cluster
---

# Сервер отладки кластера остаётся включённым до `--no-debug`

`cluster start --debug` и `cluster restart --debug` поднимают агента с сервером отладки HTTP на петле и закрепляют
это вместе с портами кластера: следующие `start` и `restart` без ключа поднимают его так же, пока `--no-debug` не
снимет признак. Признак закрепляется только после успешного запуска агента: отказанный или упавший запуск оставляет
прежний, и следующий `start` без ключа поднимает кластер, как было закреплено до него. Работающий кластер с другим признаком `start` не перезапускает: это отказ, называющий `cluster
restart` с нужным ключом. Закреплённое прежних версий без признака — кластер без отладки, реестр и порты те же. Занятый чужим процессом
порт отладки реестр не переносит: без отладки он не нужен, а запуск с отладкой отказывает, называя порт и процесс.
Сервер отладки кластера есть там же, где управляемый кластер, — на Windows и Linux
(`INV.CLI.MANAGED-SERVICES-RUN-ON-WINDOWS-AND-LINUX`); на Linux он замерен вживую в #768. Apache кластера порт его
сервера отладки не занимает.
