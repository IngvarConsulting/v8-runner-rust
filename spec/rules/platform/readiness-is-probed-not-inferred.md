---
id: INV.PLATFORM.READINESS-IS-PROBED-NOT-INFERRED
check:
  - tests/cli_dump_agent.rs::managed_agent_dumps_through_the_built_in_ssh_client_and_reads_the_result_from_disk
  - tests/cli_dump_agent.rs::a_rejected_password_is_an_environment_refusal_even_though_the_port_answers
  - tests/cli_dump_agent.rs::an_unreachable_attached_agent_is_refused_and_no_process_is_launched_instead
  - tests/cli_agent_standalone.rs::a_gate_that_presents_another_key_is_refused_by_name
  - tests/cli_agent_standalone.rs::gate_create_reads_languages_before_its_only_mutation
  - src/platform/edt_session.rs::baseline_probe_mismatch_restarts_and_drains_queue
  - src/platform/designer.rs::only_a_ready_direct_gate_dispatches_the_designer_request
  - tests/cli_standalone_direct_gate.rs::unready_direct_gate_refuses_before_dispatching_the_request
  - src/use_cases/cluster_admin.rs::a_foreign_listener_prevents_the_rac_probe
  - src/use_cases/cluster_admin.rs::command_ras_discards_a_mismatched_candidate_before_trying_the_exact_version
---

# Готовность проверяется перед использованием, а не выводится

Каждый путь проверяется своей пробой непосредственно перед работой; из флагов запуска чужого процесса готовность не выводится.

| Путь | Проба |
| --- | --- |
| Агент `managed` | SSH с проверкой ключа поднятого процесса, аутентификацией и открытием shell |
| Агент `attached` | SSH-аутентификация с настройками подключаемого агента и открытие shell |
| Агент `gate` | SSH-аутентификация шлюза, открытие shell и `common connect-ib` |
| Общая сессия EDT (MCP) | Приглашение CLI при запуске, проверка рабочего каталога при восстановлении исходного состояния |
| Прямой шлюз автономного сервера | Ответ Конфигуратора на `/GetConfigGenerationID` с теми же адресом и реквизитами ИБ |
| RAS раннера | Слушатель принадлежит запущенному процессу, `rac cluster list` подтверждает доступ к агенту кластера и возвращает разбираемый список |

Повторное использование живого агента сохраняет установленное соединение по правилу
[жизни агента](a-live-agent-stays-connected-between-commands.md).
