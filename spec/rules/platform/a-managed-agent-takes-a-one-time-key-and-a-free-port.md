---
id: INV.PLATFORM.A-MANAGED-AGENT-TAKES-A-ONE-TIME-KEY-AND-A-FREE-PORT
check:
  - src/platform/agent.rs::a_one_time_host_key_lives_with_its_owner_and_sweeps_orphans
  - src/platform/agent.rs::launch_args_carry_only_the_agent_keys_and_the_infobase_address
  - tests/cli_infobase.rs::a_taken_port_of_the_managed_agent_is_named
  - tests/cli_infobase.rs::a_managed_agent_that_exits_before_a_session_is_refused_at_once
---

# Конфигуратор принимает одноразовый ключ и свободный порт раннера

Конфигуратор в агентском режиме принимает ED25519-ключ, созданный раннером, и публикует его
отпечаток; принимает свободный порт, выбранный раннером, в `/AgentPort`. На занятый
`/AgentPort` агент отвечает выходом с кодом 0, а причину пишет только в `/Out`
(macOS, Linux под root и Windows); поведение на нечитаемом ключе замерено только на macOS.
Раннер передаёт агенту `/Out` и считает выход процесса до первой
сессии отказом: не ждёт срока подъёма, называет порт занятым, если после выхода его держит
другой процесс, и приводит текст `/Out`. Слушатель на порту, который не отвечает по SSH, ожидание
не держит. Ключ, порт, ранний отказ и наследование Windows ACL подтверждены
[замером 07.11.2026](../../../references/1c/confirmed-runtime-measurements.md#управляемый-агент-ключ-порт-и-windows-acl-на-linux-и-windows-429)
на 8.3.27 и 8.5.1; в том же файле — предыдущий macOS-замер 08.10.2026 на 8.3.27 и 8.5.4.
