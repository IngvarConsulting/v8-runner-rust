---
id: INV.CONFIG.A-STANDALONE-EXECUTOR-NEEDS-ITS-DECLARED-WAY
check:
  - tests/cli_standalone_direct_gate.rs::without_the_ssh_gate_the_agent_is_not_offered
  - tests/cli_agent_standalone.rs::without_the_direct_gate_a_standalone_server_has_only_the_agent
  - tests/cli_agent_standalone.rs::a_standalone_snapshot_without_the_direct_gate_is_refused_before_any_session
  - tests/cli_agent_standalone.rs::a_download_of_the_database_configuration_is_refused_before_the_gate
---

# Исполнителю автономной цели нужен объявленный путь

Конфигуратор идёт к автономному серверу строкой прямого шлюза в `connection`, агент —
SSH-шлюзом `standalone.gate`. Исполнитель, путь которого не объявлен, в цепочку умолчаний не
входит и пропущенным в квитанции не называется. Ключ `providers.*`, назначивший такого
исполнителя, — ошибка валидации, которая называет ключ пути. Операция, у всех исполнителей
которой нет объявленного пути, отказывает ошибкой валидации до запуска платформы и до сессии
шлюза и называет, что объявить.
