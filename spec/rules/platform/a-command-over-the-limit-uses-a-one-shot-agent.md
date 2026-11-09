---
id: INV.PLATFORM.A-COMMAND-OVER-THE-LIMIT-USES-A-ONE-SHOT-AGENT
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/577
---

# Команда сверх предела работает с разовым агентом

Обычная команда, которой нужен агент, не отказывает, когда живых агентов уже столько,
сколько допускает `tools.designer_agent.live-max`, и снять некого
(`INV.CLI.AN-AGENT-HOLD-OVER-THE-LIMIT-IS-REFUSED`). Она поднимает агента только для себя и после
работы снимает его, как раньше. В квитанции у неё `kept: false` и `reason: limit`
(`INV.WIRE.A-RECEIPT-SAYS-WHETHER-THE-AGENT-WAS-LIVE`). Отказывает сверх предела только
`agent hold`.
