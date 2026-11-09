---
id: INV.WIRE.A-RECEIPT-SAYS-WHETHER-THE-AGENT-WAS-LIVE
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/577
---

# Квитанция говорит, был ли агент живым

Если команда работала через управляемого агента, в `provider.endpoint` есть поле `live`:

- `reused`: `true`, если команда взяла уже живого агента, и `false`, если подняла его сама;
- `holders`: держатели на момент ответа. Держатель с процессом записан как `pid` и
  `started_at`, держатель со сроком — как `until`. Если держателей нет, список пустой;
- `kept`: остался ли агент жить после команды;
- `reason`: если не остался — почему (`live_agent_off`, `limit`, `interrupted`,
  `launch_mode`).

У подключённого агента, шлюза, превью и отказа до подключения поля `live` нет.
