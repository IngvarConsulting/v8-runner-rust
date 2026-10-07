---
id: INV.USE-CASES.A-CLUSTER-ROW-DOES-NOT-NAME-IBCMD
check:
  - src/domain/capability.rs::a_cluster_row_names_ibcmd_only_where_it_does_not_reach_the_cluster_infobase
  - tests/cli_build.rs::a_cluster_infobase_refuses_providers_push_ibcmd_before_the_platform
  - tests/cli_infobase.rs::a_complete_server_dbms_contract_does_not_bring_ibcmd_into_a_cluster_download
  - src/use_cases/configure_extensions.rs::extensions_on_a_cluster_target_go_to_the_agent_not_to_ibcmd
  - tests/provider_matrix.rs::a_cluster_base_refuses_ibcmd_for_every_operation_it_left
---

# В строках кластера нет `ibcmd`

У кластерной цели `ibcmd` не стоит ни в цепочке умолчаний, ни среди исполнителей, которых
назначает ключ `providers.*`. Исключения — `make` и `convert`, которые работают во
временной базе раннера и базы проекта не касаются.

Ключ `providers.<операция>: ibcmd` у кластерной базы отказывает при проверке настроек и
называет исполнителей строки. Полная секция `dbms` и готовый `ibcmd` на машине раннера в
цепочку кластера его не вводят.

Правило говорит о строках матрицы — исполнителях операций. Вспомогательный вызов `ibcmd`
перед `upload .cfe` у кластера держит отдельное правило
`INV.USE-CASES.AN-EXTENSION-UPLOAD-ON-A-CLUSTER-ASKS-NO-IBCMD` (#431).
