---
id: INV.PLATFORM.RAS-STOPS-APART-FROM-RAGENT
check:
  - src/use_cases/managed_stand/cluster.rs::stop_takes_ras_then_the_agent_then_the_step_postgres
  - src/use_cases/managed_stand/cluster.rs::the_remnants_of_a_fallen_agent_are_stopped_by_their_pid
  - src/use_cases/managed_stand/cluster.rs::a_fallen_ras_is_raised_again_for_the_running_agent
---

# `ras` останавливается отдельно от `ragent`

`ras` переживает конец `ragent`, поэтому у него своя запись, и `cluster stop` останавливает его отдельным шагом,
первым. Свой `ras` без своего агента `cluster start` снимает, а к работающему агенту без `ras` поднимает новый.
