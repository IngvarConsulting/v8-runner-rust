---
id: INV.USE-CASES.A-CLUSTER-ROW-DOES-NOT-NAME-IBCMD
check:
  - src/domain/capability.rs::a_cluster_row_names_ibcmd_only_where_it_does_not_reach_the_cluster_infobase
---

# В строках кластера нет `ibcmd`

У кластерной цели `ibcmd` не стоит ни в цепочке умолчаний, ни среди исполнителей, которых
назначает ключ `providers.*`. Исключения — `infobase.create`,
пока серверную базу не создаёт Конфигуратор
([#204](https://github.com/IngvarConsulting/v8-runner-rust/issues/204)), и `make`, который
собирает пакет во временной базе раннера и базы проекта не касается.
