---
id: INV.WIRE.A-SESSION-RECEIPT-NAMES-ITS-ENDPOINT
check:
  - tests/architecture_guardrails.rs::every_agent_receipt_passes_through_the_session_stamp
  - tests/architecture_guardrails.rs::the_session_stamp_guard_sees_indirection_and_return_bypasses
  - tests/architecture_guardrails.rs::the_session_stamp_guard_rejects_each_real_entry_without_its_stamp
  - src/use_cases/context.rs::the_first_session_endpoint_is_shared_by_context_clones
  - tests/cli_dump_agent.rs::an_unreachable_attached_agent_is_refused_and_no_process_is_launched_instead
  - tests/cli_dump_agent.rs::an_upload_receipt_names_the_agent_used_by_its_extension_probe
  - tests/cli_agent_scenarios.rs::a_managed_agent_session_is_named_in_the_receipt
  - tests/cli_dump_agent.rs::an_attached_agent_session_is_named_in_the_receipt
  - tests/cli_agent_standalone.rs::a_gate_session_is_named_in_the_receipt
  - tests/cli_agent_scenarios.rs::a_receipt_without_a_session_has_no_endpoint
  - src/use_cases/agent_session.rs::a_receipt_address_never_carries_credentials
  - tests/cli_standalone_direct_gate.rs::a_standalone_server_with_only_the_direct_gate_is_served_by_the_designer
---

# Квитанция сессии называет точку входа

Там, где операция шла через сессию агента, квитанция исполнителя несёт `provider.endpoint`:
`mode` — `managed`, `attached` или `gate` — и `address`, `host:port` того подключения,
которое команда открыла. У управляемого агента это `127.0.0.1` и его порт, у шлюза
автономного сервера — адрес шлюза. Учётных данных в адресе нет. У превью, отказа до
подключения и команды, исполненной только процессом платформы, в том числе
Конфигуратором по прямому шлюзу автономного сервера, поля нет.
