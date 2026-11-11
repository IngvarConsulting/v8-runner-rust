---
id: INV.WIRE.A-RECEIPT-SAYS-WHETHER-THE-AGENT-WAS-LIVE
check:
  - src/use_cases/context.rs::the_live_agent_receipt_lands_in_the_same_session_slot
  - tests/cli_live_agent.rs::a_second_command_reuses_the_live_agent
  - tests/cli_live_agent.rs::live_off_removes_the_live_agent_and_works_one_shot
  - tests/cli_live_agent.rs::over_the_limit_an_idle_agent_gives_way_and_a_held_one_does_not
  - tests/cli_live_agent.rs::an_interrupted_command_removes_the_live_agent
  - src/use_cases/agent_session.rs::the_receipt_says_removed_only_when_the_keeper_removes_the_agent
  - src/use_cases/agent_session.rs::a_busy_live_agent_makes_the_command_one_shot_with_reason_busy
  - src/use_cases/agent_session.rs::a_failed_request_names_its_own_reason
  - tests/cli_live_agent_portable.rs::a_receipt_says_removed_when_the_last_holder_left_during_the_command
  - tests/cli_live_agent_portable.rs::status_deep_reports_reusing_the_generation_agent
---

# Квитанция говорит, был ли агент живым

Если команда работала через управляемого агента, в `provider.endpoint` есть поле `live`:

- `reused`: `true`, если команда взяла уже живого агента, и `false`, если подняла его сама;
- `holders`: держатели на момент ответа. Держатель с процессом записан как `pid` и
  `started_at`, держатель со сроком — как `until`. Если держателей нет, список пустой;
- `kept`: остался ли агент жить после команды;
- `reason`: если не остался — почему: `live_agent_off` (живой агент выключен), `limit` (предел
  живых агентов), `job` (Job без права выхода), `interrupted` (команду прервали посреди
  запроса агенту), `busy` (живой агент рабочей копии ещё служит сессии другой команды),
  `keeper_failed` (сторож не поднялся или не ответил) или `removed` (сторож снял агента после
  команды: его остановили, его сессия оборвалась или ушёл последний держатель). С `busy` и
  `keeper_failed` команда не отказывает, а работает с разовым агентом. `kept` называет решение
  сторожа после конца сессии команды, а не надежду команды.

Поле `live` у всех форм лежит в одном месте: в `endpoint` той квитанции исполнителя
`provider`, через которую шла работа агента. Это `data.provider`, а у команд из нескольких
шагов — `provider` каждого шага. Его состав одинаков во всех формах.

У подключённого агента, шлюза, превью и отказа до подключения поля `live` нет.
