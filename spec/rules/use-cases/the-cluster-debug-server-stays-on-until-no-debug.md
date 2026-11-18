---
id: INV.USE-CASES.THE-CLUSTER-DEBUG-SERVER-STAYS-ON-UNTIL-NO-DEBUG
check:
  - src/use_cases/managed_stand/cluster.rs::a_debug_start_raises_the_agent_with_its_debug_server
  - src/use_cases/managed_stand/cluster.rs::the_debug_server_stays_on_until_no_debug
  - src/use_cases/managed_stand/cluster.rs::a_debug_start_on_a_cluster_running_without_it_is_refused
  - src/use_cases/managed_stand/cluster.rs::pins_written_before_the_debug_server_keep_their_registry
---

# Сервер отладки кластера остаётся включённым до `--no-debug`

`cluster start --debug` и `cluster restart --debug` поднимают агента с сервером отладки HTTP на петле и закрепляют
это вместе с портами кластера: следующие `start` и `restart` без ключа поднимают его так же, пока `--no-debug` не
снимет признак. Работающий кластер с другим признаком `start` не перезапускает: это отказ, называющий `cluster
restart` с нужным ключом. Закреплённое прежних версий без признака — кластер без отладки, реестр и порты те же.
Управляемый кластер есть только на Windows (`INV.CLI.MANAGED-SERVICES-RUN-ON-WINDOWS`).
