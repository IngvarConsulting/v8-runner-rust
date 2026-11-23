---
id: INV.USE-CASES.THE-CLUSTER-DEBUG-SERVER-STAYS-ON-UNTIL-NO-DEBUG
check:
  - src/use_cases/managed_stand/cluster.rs::a_new_cluster_starts_with_debugging_unless_explicitly_disabled
  - src/use_cases/managed_stand/cluster.rs::a_failed_first_no_debug_start_does_not_override_the_default
  - src/use_cases/managed_stand/cluster.rs::a_move_whose_new_agent_does_not_stop_is_left_to_the_next_start
  - src/use_cases/managed_stand/cluster.rs::a_debug_start_raises_the_agent_with_its_debug_server
  - src/use_cases/managed_stand/cluster.rs::the_debug_server_stays_on_until_no_debug
  - src/use_cases/managed_stand/cluster.rs::a_debug_start_on_a_cluster_running_without_it_is_refused
  - src/use_cases/managed_stand/cluster.rs::pins_written_before_the_debug_server_keep_their_registry
  - src/use_cases/managed_stand/cluster.rs::a_taken_debug_port_moves_no_registry
  - src/use_cases/managed_stand/cluster.rs::a_refused_debug_start_pins_no_debug
  - src/use_cases/managed_stand/web.rs::apache_never_takes_the_debug_port_of_the_cluster
---

# Сервер отладки кластера остаётся включённым до `--no-debug`

Управляемый кластер без закреплённого режима на Windows и Linux по умолчанию поднимает агента с сервером
отладки HTTP на петле. `--debug` явно включает отладку, `--no-debug` явно отключает. После успешного запуска
выбор сохраняется вместе с портами. Следующие `start` и `restart` без ключа используют сохранённый режим.
Неудачный первый запуск сохраняет умолчание, а не запрошенное исключение. Неудачный последующий запуск
сохраняет прежний режим. Работающий кластер с другим признаком `start` не перезапускает: это отказ, называющий `cluster
restart` с нужным ключом. Закреплённое прежних версий без признака — кластер без отладки, реестр и порты те же. Занятый чужим процессом
порт отладки реестр не переносит: без отладки он не нужен, а запуск с отладкой отказывает, называя порт и процесс.
Сервер отладки кластера есть там же, где управляемый кластер, — на Windows и Linux
(`INV.CLI.MANAGED-SERVICES-RUN-ON-WINDOWS-AND-LINUX`); на Linux он замерен вживую в #768. Apache кластера порт его
сервера отладки не занимает.
