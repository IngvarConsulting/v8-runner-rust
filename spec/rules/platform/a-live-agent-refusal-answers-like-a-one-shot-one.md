---
id: INV.PLATFORM.A-LIVE-AGENT-REFUSAL-ANSWERS-LIKE-A-ONE-SHOT-ONE
check:
  - src/platform/agent_keeper/protocol.rs::an_environment_layer_and_a_kind_survive_the_keeper_channel
  - src/platform/agent_keeper/protocol.rs::an_agent_failure_keeps_its_kind_across_the_channel
  - src/platform/agent_keeper/protocol.rs::a_launch_or_keeper_failure_travels_as_other_with_its_text
---

# Отказ живого агента отвечает тем же кодом, что у разового

Отказ агента, который сторож живого агента передаёт команде по своему каналу, приходит к ней
тем же родом и кодом ответа и с тем же `error.layer`, что и у разового агента: отмена — отменой,
брошенная команда — `agent_command_abandoned`, отказ платформы — `platform_failure`, отказ
окружения — со своим слоем (`INV.WIRE.AN-ENVIRONMENT-REFUSAL-NAMES-ITS-LAYER`). Канал
переносит отказ агента его собственным видом или видом того же рода и слоя — тогда может
отличаться текст; отказом сторожа становятся только отказы подъёма агента и самого сторожа.

Перенос закреплён для каждого вида отказа агента, в том числе для тех, которых сторож сегодня
не порождает: отказа отменённой до отправки команды, брошенной команды, обмена файлами и
каталога пользователя агента. Поэтому ответ команды не изменится и тогда, когда сторож начнёт
их порождать.
