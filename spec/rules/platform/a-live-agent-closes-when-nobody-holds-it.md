---
id: INV.PLATFORM.A-LIVE-AGENT-CLOSES-WHEN-NOBODY-HOLDS-IT
check:
  - tests/agent_keeper.rs::an_idle_agent_closes_after_its_idle_limit
  - tests/agent_keeper.rs::status_and_hold_answer_during_a_long_run
  - tests/agent_keeper.rs::an_open_session_keeps_the_agent_from_idling_out
---

# Живой агент без держателя закрывается после 15 минут простоя

Живого агента без держателя отпускают и снимают
(`INV.PLATFORM.A-MANAGED-AGENT-IS-RELEASED-AND-REMOVED`), если он простаивает 15 минут.
Простоем считается только время, когда агент не выполняет ничью команду, поэтому длинная
команда простоем не считается, сколько бы она ни шла. Простой отсчитывается от конца
последней команды. Каждая новая команда его сбрасывает.

После того как последний держатель отпустил агента (`v8-runner agent release`), простой
отсчитывается с этого момента. Пока держатель есть, простой не ограничен.
