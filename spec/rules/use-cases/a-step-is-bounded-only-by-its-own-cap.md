---
id: INV.USE-CASES.A-STEP-IS-BOUNDED-ONLY-BY-ITS-OWN-CAP
check:
  - src/use_cases/context.rs::a_step_policy_carries_the_steps_own_cap_and_nothing_above_it
  - src/use_cases/context.rs::the_operators_interrupt_is_the_only_command_boundary_interruption
  - tests/architecture_guardrails.rs::a_command_carries_no_deadline_anywhere_it_could_be_put_back
---

# Шаг ограничен только собственным пределом

Шаг получает ровно тот предел, который объявил сам. Объявивший `None` идёт до
терминального исхода; сверху ни команда, ни вложенная оркестрация предела не
добавляют. Ожидание допуска у сервера MCP предел шага не укорачивает.
