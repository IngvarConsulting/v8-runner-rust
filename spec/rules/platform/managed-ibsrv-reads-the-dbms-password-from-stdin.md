---
id: INV.PLATFORM.MANAGED-IBSRV-READS-THE-DBMS-PASSWORD-FROM-STDIN
check:
  - src/platform/ibsrv.rs::the_dbms_password_goes_through_stdin_and_never_through_an_argument
  - src/platform/service.rs::a_service_reads_what_was_given_to_its_stdin_and_the_argument_stays_clean
---

# Пароль СУБД уходит в `ibsrv` через stdin

Долгоживущий `ibsrv` над базой в СУБД получает пароль ключом `-W` через stdin, а в его командной строке пароля нет.
Короткий `ibcmd infobase create` несёт его в аргументах, как `CREATEINFOBASE` и `rac`.
