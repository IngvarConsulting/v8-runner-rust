---
id: INV.PLATFORM.MANAGED-PORTS-ARE-PINNED
check:
  - src/use_cases/managed_stand/mod.rs::the_first_start_creates_the_data_directory_and_the_second_reuses_it
  - src/use_cases/managed_stand/mod.rs::a_foreign_process_on_the_pinned_port_is_named_and_left_alone
  - src/use_cases/managed_stand/cluster.rs::the_ports_and_the_registry_stay_from_start_to_start
  - src/use_cases/managed_stand/cluster.rs::a_lost_pins_file_does_not_remove_the_old_registry
---

# Порт, на который ссылаются данные, закреплён

Порт PostgreSQL записан в зарегистрированной базе, а `regport`, порт агента и диапазон рабочих процессов — в реестре
кластера: запуск с прежним реестром и другими `/port` и `/range` оставляет в нём прежние значения (замер #538). Порт
`ras` в реестр не попадает и выбирается свободным при каждом запуске. Поэтому они закрепляются при первом
запуске и хранятся отдельно от записи о запущенной службе, так что её снятие и замена их не отменяют. Занятый
закреплённый порт PostgreSQL — отказ с названием процесса. Если закреплённое кластера потеряно, следующий запуск берёт
новый блок и новый реестр, а прежние реестры не удаляет: в одном из них может быть зарегистрирована база. Переноса порта PostgreSQL команды не делают: работа базы после
него не подтверждена.
