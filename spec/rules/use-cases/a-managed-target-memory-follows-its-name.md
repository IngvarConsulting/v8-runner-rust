---
id: INV.USE-CASES.A-MANAGED-TARGET-MEMORY-FOLLOWS-ITS-NAME
check:
  - src/config/loader.rs::a_managed_standalone_without_a_running_server_has_no_address_and_keeps_its_memory_by_name
  - src/config/loader.rs::infobase_create_sees_the_managed_standalone_as_its_file_base_with_the_same_memory
  - src/use_cases/managed_stand/cluster.rs::infobase_create_targets_the_running_cluster_with_the_dbms_access
---

# Память управляемой цели привязана к её имени

Адрес управляемой цели в памяти раннера — имя объявленной базы, а не `Srvr=`. Смена портов и перенос кластера на
новый `regport` память не сбрасывают: данные и поколение конфигурации остаются прежними.
