---
id: INV.USE-CASES.THE-AGENT-LEADS-A-DEFAULT-CHAIN
check:
  - src/domain/capability.rs::the_agent_leads_the_file_and_cluster_chains
  - tests/contract_receipt.rs::every_operation_with_an_executor_answers_with_a_receipt
---

# Агент стоит первым в цепочках файловой базы и кластера

У `push`, `pull` и `download` цепочка умолчаний файловой базы — `agent`, `designer`,
`ibcmd`, у кластера — `agent`, `designer`. У `infobase dump` и `infobase restore` цепочка
файловой базы и кластера — `agent`, `designer`. Агент ни в одной строке матрицы не помечен
экспериментальным.
