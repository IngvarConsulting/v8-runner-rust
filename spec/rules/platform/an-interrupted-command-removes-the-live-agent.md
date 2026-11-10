---
id: INV.PLATFORM.AN-INTERRUPTED-COMMAND-REMOVES-THE-LIVE-AGENT
check:
  - tests/agent_keeper.rs::a_client_leaving_in_the_middle_of_a_command_removes_the_agent
  - src/platform/agent_keeper/client.rs::a_cancelled_run_leaves_the_channel_and_the_keeper_sees_it
  - tests/cli_live_agent.rs::an_interrupted_command_removes_the_live_agent
  - src/platform/agent_keeper/client.rs::a_run_past_its_deadline_is_cancelled
  - src/platform/agent_keeper/client.rs::a_critical_run_past_its_deadline_is_finished_and_the_deadline_is_deferred
---

# Прерванная команда снимает живого агента

Если команду прервали, пока агент выполнял её запрос (отмена, срок, отказ сессии), раннер
снимает живого агента, а не оставляет его следующей команде. Иначе следующая команда взяла
бы агента, который ещё выполняет чужую работу, например `update-db-cfg`. Так же поступает
следующая команда, если находит агента своего ключа без живой команды-владельца, а его
запись говорит, что он занят: например, когда прежнюю команду сняли `kill -9`.
Держатели агента остаются, и следующая команда поднимает им нового агента.
