---
id: INV.PLATFORM.A-LIVE-AGENT-REFUSAL-ANSWERS-LIKE-A-ONE-SHOT-ONE
check:
  - src/platform/agent_keeper/protocol.rs::an_environment_layer_and_a_kind_survive_the_keeper_channel
  - src/platform/agent_keeper/protocol.rs::an_agent_failure_keeps_its_kind_across_the_channel
  - src/platform/agent_keeper/protocol.rs::an_unknown_failure_kind_from_another_keeper_is_read_as_a_keeper_failure
  - src/platform/agent_keeper/protocol.rs::a_failure_folded_into_other_by_an_older_keeper_is_still_read
---

# Отказ живого агента отвечает тем же кодом, что у разового

Отказ агента, который сторож живого агента передаёт команде по своему каналу, приходит к ней
тем же родом и кодом ответа и с тем же `error.layer`, что и у разового агента: отмена — отменой,
брошенная команда — `agent_command_abandoned`, отказ платформы — `platform_failure`, отказ
окружения — со своим слоем (`INV.WIRE.AN-ENVIRONMENT-REFUSAL-NAMES-ITS-LAYER`). Каждый вид
отказа агента переносится каналом своим видом; отказом сторожа становятся только отказы
подъёма агента и самого сторожа. Текст отказа может отличаться: часть видов канал сводит к
виду того же рода.

Вид отказа, которого сборка не знает, кадр не ломает: он читается отказом сторожа. Так же
читается и прежняя запись `other`, которой сторож прежней сборки передавал эти отказы.
