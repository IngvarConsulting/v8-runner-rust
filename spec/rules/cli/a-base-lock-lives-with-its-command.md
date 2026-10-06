---
id: INV.CLI.A-BASE-LOCK-LIVES-WITH-ITS-COMMAND
check:
  - tests/cli_infobase_lock.rs::a_base_held_by_a_killed_command_lets_the_next_one_in
  - tests/cli_infobase_lock.rs::launch_releases_the_base_lock_while_its_client_runs
---

# Замок базы живёт, пока жива команда

Замок файловой базы, оставленный убитой командой, следующую команду не останавливает.

Источник: [`architecture.html#life`](../../../docs/site/architecture.html#life).
