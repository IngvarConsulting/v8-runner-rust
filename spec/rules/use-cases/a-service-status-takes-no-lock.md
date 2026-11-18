---
id: INV.USE-CASES.A-SERVICE-STATUS-TAKES-NO-LOCK
check:
  - tests/cli_dbms.rs::status_answers_while_another_command_holds_the_work_path
  - tests/cli_web.rs::web_status_of_a_declared_cluster_writes_nothing
---

# Состояние службы читается без замка

`status` читает запись без замка `workPath`, чтобы отвечать и пока идёт другая команда.
