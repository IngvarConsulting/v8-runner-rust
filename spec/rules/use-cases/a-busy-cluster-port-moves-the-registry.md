---
id: INV.USE-CASES.A-BUSY-CLUSTER-PORT-MOVES-THE-REGISTRY
check:
  - src/use_cases/managed_stand/cluster.rs::a_busy_regport_moves_the_cluster_to_a_new_registry_over_its_database
  - src/use_cases/managed_stand/cluster.rs::a_busy_agent_or_worker_port_moves_the_cluster_to_a_new_registry
  - src/use_cases/managed_stand/cluster.rs::restart_moves_a_running_cluster_off_a_taken_port
  - src/use_cases/managed_stand/cluster.rs::a_new_registry_keeps_clear_of_the_port_of_the_runners_postgres
  - src/use_cases/managed_stand/cluster.rs::no_free_block_for_a_new_registry_is_refused_and_the_ports_stay
  - src/use_cases/managed_stand/cluster.rs::a_foreign_agent_over_this_registry_is_refused_without_a_new_registry
  - src/use_cases/managed_stand/cluster.rs::new_regport_moves_the_base_to_a_new_registry_over_its_database
  - src/use_cases/managed_stand/cluster.rs::a_failed_move_keeps_the_old_registry_and_its_ports
  - src/use_cases/managed_stand/cluster.rs::new_regport_on_free_ports_is_refused
  - src/use_cases/managed_stand/cluster.rs::the_refusals_of_a_busy_port_come_before_the_postgres_starts
  - src/use_cases/managed_stand/cluster.rs::a_taken_port_without_the_process_list_is_refused_and_nothing_is_removed
  - src/use_cases/managed_stand/cluster.rs::start_rolls_back_an_interrupted_move
  - src/use_cases/managed_stand/cluster.rs::a_move_whose_new_agent_does_not_stop_is_left_to_the_next_start
  - src/use_cases/managed_stand/cluster.rs::a_foreign_agent_over_this_registry_on_other_ports_is_refused
  - src/use_cases/managed_stand/cluster.rs::a_pinned_block_over_the_port_of_the_runners_postgres_moves_to_a_new_registry
  - src/use_cases/managed_stand/cluster.rs::a_ras_port_over_the_port_of_the_runners_postgres_takes_a_new_port
  - src/use_cases/managed_stand/cluster.rs::the_port_of_the_postgres_in_the_block_keeps_the_refusals_of_a_busy_port
---

# Занятый закреплённый порт кластера ведёт к новому реестру, живой реестр — к отказу

Если закреплённый порт — `regport`, порт агента или порт диапазона рабочих процессов — занят чужим процессом,
`cluster start` и `restart` сами делают то же, что `--new-regport`: заводят новый реестр на новых портах, не задевая
прежние и порт PostgreSQL раннера. Чужой процесс остаётся работать. Ответ называет `regport` прежнего реестра в
`previous_regport` и предупреждает, указав порт и pid занявшего.

Так же — новым реестром — `cluster start` и `restart` уходят из закреплённого блока, в который попал закреплённый порт
PostgreSQL раннера: поднятый, PostgreSQL занял бы порт кластера, или кластер — его порт. Сверка идёт до запуска
PostgreSQL, одной проверкой с автономным сервером; предупреждение называет порт PostgreSQL раннера. Порт `ras` не
закрепляется: запуск берёт свободный вне блока кластера и порта PostgreSQL раннера, так что `ras` на порту PostgreSQL
получает новый порт, а блок кластера остаётся.

Остаётся отказом `workspace_busy` без нового реестра: на порту или на других портах работает чужой `ragent` над
реестром этой копии — такой реестр живой, — или порт занят, а перечня процессов нет, и кто его держит, не сказать;
тогда ничего не удаляется. Эти отказы приходят до запуска PostgreSQL. Нет свободного блока — отказ среды.
`--new-regport` при свободных портах — отказ.

Новый реестр регистрирует объявленную базу без `--create-database` над прежней БД: UUID базы в реестре меняется,
данные и поколение конфигурации остаются. Была ли база, раннер знает по признаку создания в прежнем реестре, а у своего
PostgreSQL — и по самой БД. Если базы ещё нет, остаётся только новый реестр. Прежний реестр удаляется после регистрации
и успешной пробы базы, поэтому отказ на этом шаге ничего не теряет. Закреплённое помнит прежний реестр, пока перенос не
закончен: прерванный перенос следующий `cluster start` откатывает к прежним портам, а рабочие команды и
`infobase create` такой кластер не видят. Если агент нового реестра при неудаче не снялся, перенос так и остаётся
записанным до следующего `cluster start`.

Адрес кластера живёт только в его состоянии под `localPath`. Файлы `v8project*.yaml` раннер при переносе не правит, и
адрес у `origin` не пишется: адрес берут из ответа `start` или `status`, а не кешируют.
