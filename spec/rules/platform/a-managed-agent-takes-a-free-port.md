---
id: INV.PLATFORM.A-MANAGED-AGENT-TAKES-A-FREE-PORT
check:
  - tests/cli_bootstrap.rs::bootstrap_empty_dir_creates_config_and_dumps_main_configuration
  - tests/cli_dump_agent.rs::managed_agent_dumps_through_the_built_in_ssh_client_and_reads_the_result_from_disk
  - tests/cli_build_agent.rs::a_managed_build_loads_and_updates_in_one_session_and_records_the_generation
  - tests/cli_infobase.rs::a_taken_port_of_the_managed_agent_is_named
---

# Порт управляемого агента по умолчанию — свободный на этот запуск

Без `tools.designer_agent.port` раннер берёт у системы свободный порт на `127.0.0.1` и
передаёт его агенту в `/AgentPort`; фиксированного порта по умолчанию нет. Объявленный порт
передаётся как есть. Если агент не поднялся, а его порт занят другим процессом, отказ
`environment_unavailable` называет порт занятым.
