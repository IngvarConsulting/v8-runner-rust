---
id: INV.USE-CASES.A-STANDALONE-START-NAMES-ITS-SERVER
check:
  - src/use_cases/managed_stand/standalone.rs::start_runs_the_server_and_names_its_addresses
  - src/use_cases/managed_stand/standalone.rs::a_second_start_names_the_running_server_and_starts_nothing
---

# `standalone start` называет поднятый сервер

Команда поднимает `ibsrv` базы из `standalone.managed` и отвечает, когда прямой шлюз принял соединение. Уже
работающий свой сервер она называет и нового не запускает.
