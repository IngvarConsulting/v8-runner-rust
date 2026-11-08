---
id: INV.PLATFORM.A-CLUSTER-LEAVES-NO-REMNANTS
check:
  - src/use_cases/managed_stand/cluster.rs::the_remnants_of_a_fallen_agent_are_stopped_by_their_pid
  - src/use_cases/managed_stand/cluster.rs::start_after_a_fallen_agent_clears_its_remnants_and_keeps_the_regport
  - src/use_cases/managed_stand/cluster.rs::a_process_with_another_parent_or_an_earlier_start_is_no_remnant
  - src/use_cases/managed_stand/cluster.rs::a_worker_raised_by_a_remnant_is_a_remnant_too
  - src/use_cases/managed_stand/cluster.rs::a_child_of_a_process_that_got_the_dead_agents_pid_is_no_remnant
  - src/use_cases/managed_stand/cluster.rs::a_child_on_another_machine_or_of_another_image_is_no_remnant
  - src/use_cases/managed_stand/cluster.rs::a_remnant_that_does_not_stop_keeps_the_agent_record
  - src/use_cases/managed_stand/cluster.rs::without_the_process_list_remnants_are_not_guessed_and_the_record_stays
---

# После кластера не остаётся его процессов

Упавший `ragent` оставляет `rmngr` и `rphost` со своими портами, а `taskkill /T` по мёртвому родителю их не снимает.
`cluster stop` и `cluster start` снимают по pid только процесс, который совпал во всём: родитель из записи мёртв,
`ParentProcessId` равен его pid или pid уже опознанного остатка (после конца агента `rmngr` поднимает новые `rphost`),
имя образа `rmngr` или `rphost`, процесс начался не раньше записанного старта `ragent`, а хеш `machine_id` записи
совпадает с машиной. Если pid агента система уже отдала новому процессу, прямой потомок по этому pid — остаток, только
если начался раньше нового владельца pid. `cluster status` такие процессы называет и не трогает. Запись агента
остаётся, пока не сняты все его остатки: только по ней их и узнать. Без перечня процессов остатки не угадываются:
команда отказывает, и запись остаётся. Занятость `regport` таким остатком не считается чужой и к `--new-regport`
не ведёт.
