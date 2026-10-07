---
id: INV.USE-CASES.DECLARED-DBMS-ACCESS-CREATES-THE-SERVER-INFOBASE
check:
  - tests/cli_init.rs::a_cluster_base_is_created_by_the_designer_with_the_client_server_string
  - tests/cli_init.rs::a_cluster_base_without_a_required_dbms_field_is_refused_before_the_platform_starts
  - tests/cli_init.rs::init_non_file_connection_keeps_running_workspace_step_and_returns_payload
  - tests/cli_init.rs::init_designer_non_zero_create_exit_stays_fatal_even_when_marker_appears
---

# Объявленный доступ к СУБД создаёт серверную базу

При полном контракте `infobase.dbms` — вид СУБД, сервер, имя базы данных и `locale` —
команда создания создаёт серверную информационную базу; без него шаг не пропускается молча, а
отказывает до запуска платформы и называет недостающий ключ. Отдельного ключа для создания
нет.

Итог шага определяется по коду выхода платформы и наблюдаемому состоянию базы, а не по тексту
сообщения утилиты: ненулевой код выхода остаётся фатальным, даже если маркер появился.
