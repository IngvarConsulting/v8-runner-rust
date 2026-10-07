---
id: INV.PLATFORM.A-MANAGED-AGENT-TAKES-A-ONE-TIME-KEY-AND-A-FREE-PORT
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/429
---

# Конфигуратор принимает одноразовый ключ и свободный порт раннера

Конфигуратор в агентском режиме принимает ED25519-ключ, созданный раннером, и публикует его
отпечаток; принимает свободный порт, выбранный раннером, в `/AgentPort`; на занятый
`/AgentPort` отвечает выходом, по которому раннер отказывает с причиной «порт занят». Замер
с датой, сборкой и ОС записан в `references/1c/confirmed-runtime-measurements.md`.
