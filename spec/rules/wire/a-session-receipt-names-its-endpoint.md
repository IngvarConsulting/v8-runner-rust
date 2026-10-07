---
id: INV.WIRE.A-SESSION-RECEIPT-NAMES-ITS-ENDPOINT
check:
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
автономного сервера — адрес шлюза. Учётных данных в адресе нет. Команда, которая сессии не
открывала, — процесс платформы, в том числе Конфигуратор по прямому шлюзу автономного
сервера, превью, отказ до подключения — поля не несёт.
