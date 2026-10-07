---
id: INV.USE-CASES.A-MANAGED-AGENT-THAT-DID-NOT-START-IS-NOT-REPLACED
check:
  - tests/cli_infobase.rs::a_managed_agent_that_did_not_start_fails_the_command_without_the_designer
---

# Неподнявшийся управляемый агент — отказ, а не откат на Конфигуратор

Готовность управляемого агента — найденная платформа, и выбор исполнителя кончается до его
запуска. Если агент после этого не поднялся, команда отказывает
(`environment_unavailable`), квитанция называет `selected: agent`, а Конфигуратор пакетным
процессом не вызывается. Вернуть Конфигуратор первым можно только ключом
`providers.<операция>: designer`.
