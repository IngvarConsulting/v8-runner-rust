---
id: INV.WIRE.A-BUSY-WORKSPACE-ANSWERS-WORKSPACE-BUSY
check:
  - tests/contract_workspace_busy.rs::every_leaf_taking_the_lock_answers_workspace_busy_on_a_busy_work_path
  - tests/contract_workspace_busy.rs::every_leaf_taking_the_lock_is_exercised_here
---

# Занятый каталог отвечает `workspace_busy`

Команда командной строки, которой отказал занятый рабочий каталог, отвечает кодом
`workspace_busy` рода `workspace` на шаге `workspace lock`.
