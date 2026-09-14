---
id: INV.USE-CASES.NESTED-WORK-INHERITS-THE-REMAINING-BUDGET
status: active
governs: product
decision: DEC.2026-04-20.EVERY-COMMAND-HAS-A-DEADLINE
check: [src/platform/edt.rs::interactive_dsl_reused_session_shares_timeout_budget_across_commands, src/use_cases/check_syntax.rs::syntax_edt_uses_mcp_timeout_budget_for_subprocess]
scope: [use-cases]
---

# Вложенная работа наследует остаток срока

Вложенная оркестрация получает то, что осталось от срока внешней команды, и собственного срока не заводит.
