---
id: INV.PLATFORM.A-MANAGED-AGENT-IS-RELEASED-AND-REMOVED
check:
  - tests/cli_agent_scenarios.rs::a_managed_agent_is_released_and_removed_whatever_its_version
  - tests/cli_agent_scenarios.rs::a_refused_release_keeps_the_outcome_and_still_removes_the_agent
  - tests/cli_build_agent.rs::a_managed_build_loads_and_updates_in_one_session_and_records_the_generation
  - src/platform/process.rs::removing_a_managed_process_ends_its_whole_job_without_waiting
  - src/platform/process.rs::removing_a_managed_process_kills_its_group_at_once_without_a_graceful_wait
---

# Свой агент отпускают и снимают, не дожидаясь его выхода

Агента, которого раннер поднял сам, отпускают так же, как чужого. Это происходит после работы, если живой агент выключен, или когда живой агент больше никому не нужен (`INV.PLATFORM.A-LIVE-AGENT-CLOSES-WHEN-NOBODY-HOLDS-IT`). Отпускают так: служебной
командой `common disconnect-ib` и закрытием сессии. Затем процесс снимают вместе с
потомками, не дожидаясь его выхода: на Windows — его job-объектом, на Unix — `SIGKILL` группы
без мягкой паузы; `common shutdown` ему не отдают — ни на какой версии. На ответ
`disconnect-ib` отведён короткий срок: молчание или отказ команды снятия процесса не
отменяют и исхода команды не меняют.

Так закрытие не зависит от того, выполняет ли агент `common shutdown`: на 8.3.24–8.3.26 он
команду не выполняет, на 8.3.27 и 8.5 выполняет за несколько секунд
([замер](../../../references/1c/confirmed-runtime-measurements.md), раздел о закрытии своего
агента). После снятия база свободна, и следующая команда Конфигуратора проходит сразу.
