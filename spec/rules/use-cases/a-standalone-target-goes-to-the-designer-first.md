---
id: INV.USE-CASES.A-STANDALONE-TARGET-GOES-TO-THE-DESIGNER-FIRST
check:
  - tests/cli_standalone_direct_gate.rs::the_designer_leads_the_chain_when_both_gates_are_declared
  - tests/cli_standalone_direct_gate.rs::the_direct_gate_serves_what_the_ssh_gate_lacks
  - tests/cli_standalone_direct_gate.rs::upload_and_restore_plan_the_designer_by_the_direct_gate
  - tests/cli_agent_standalone.rs::a_direct_gate_address_next_to_the_ssh_gate_puts_the_designer_first
  - tests/cli_agent_standalone.rs::without_the_direct_gate_a_standalone_server_has_only_the_agent
  - tests/cli_agent_standalone.rs::a_standalone_snapshot_without_the_direct_gate_is_refused_before_any_session
  - tests/cli_standalone_direct_gate.rs::without_the_ssh_gate_the_agent_is_not_offered
---

# Автономный сервер: Конфигуратор по прямому шлюзу первым, агент вторым

На автономном сервере цепочка умолчаний `push`, `pull` и `download` (включая `--state db`)
начинается с Конфигуратора, который идёт в прямой шлюз строкой из `connection` ключом
`/S <host>:<port>\<name>` с реквизитами базы, как в кластер; вторым идёт агент через
SSH-шлюз. Не готов Конфигуратор — команду исполняет агент, а квитанция называет
Конфигуратор пропущенным. `upload`, `check`, `infobase dump` и `infobase restore` исполняет
только Конфигуратор по прямому шлюзу; состав расширений — только агент.

Матрица исполнителей — `src/domain/capability.rs`, её артефакт сверяется с сайтом.
