---
id: INV.PLATFORM.MANAGED-PORTS-ARE-PINNED
check:
  - src/use_cases/managed_stand/mod.rs::the_first_start_creates_the_data_directory_and_the_second_reuses_it
  - src/use_cases/managed_stand/mod.rs::a_foreign_process_on_the_pinned_port_is_named_and_left_alone
---

# Порт, на который ссылаются данные, закреплён

Порт PostgreSQL записан в зарегистрированной базе, а `regport` — в реестре кластера. Поэтому они закрепляются при первом
запуске и хранятся отдельно от записи о запущенной службе, так что её снятие и замена их не отменяют. Занятый
закреплённый порт PostgreSQL — отказ с названием процесса. Переноса порта PostgreSQL команды не делают: работа базы после
него не подтверждена.
