---
id: INV.USE-CASES.A-STEP-IS-BOUNDED-ONLY-BY-ITS-OWN-CAP
status: active
governs: product
decision: DEC.2026-09-20.A-COMMAND-HAS-NO-DEADLINE
check: [src/use_cases/context.rs::a_step_policy_carries_the_steps_own_cap_and_nothing_above_it, src/use_cases/context.rs::the_operators_interrupt_is_the_only_command_boundary_interruption]
scope: [use-cases]
---

# Шаг ограничен только собственным пределом

Шаг получает ровно тот предел, который объявил сам. Объявивший `None` идёт до
терминального исхода; сверху ни команда, ни вложенная оркестрация предела не
добавляют.
