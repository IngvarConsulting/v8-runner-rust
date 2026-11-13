---
id: INV.USE-CASES.IBCMD-TAKES-A-DT-ONLY-WITH-EXCLUSIVE-ACCESS
check:
  - tests/cli_infobase_cross_platform.rs::ibcmd_dt_transfers_refuse_windows_during_provider_selection
  - tests/cli_infobase.rs::a_busy_ibcmd_dt_transfer_names_the_holder_and_dispatches_nothing
  - tests/cli_infobase.rs::ibcmd_dumps_dt_only_when_the_file_infobase_has_no_sessions
  - tests/cli_infobase.rs::ibcmd_restores_an_existing_or_new_file_infobase
  - tests/cli_infobase.rs::a_failed_ibcmd_dump_preserves_the_published_snapshot
  - src/platform/ibcmd.rs::a_missing_snapshot_source_or_failed_probe_never_dispatches
  - src/platform/ibcmd.rs::dt_transfers_without_a_measured_lock_probe_never_dispatch
---

# `ibcmd` снимает и загружает `.dt` только у файловой базы без сеансов

`ibcmd`, названный ключом `providers.infobase.dump|restore`, запускается для файловой базы
лишь после того, как раннер спросил ядро, держит ли кто-то файл базы `1Cv8.1CD`
(блокировку записи на весь файл, `F_GETLK`). Держит — команда отказывает до запуска `ibcmd
infobase dump` или `ibcmd infobase restore` и называет процесс-держатель; база и `.dt` не
тронуты. Не держит — снимок снимается и загружается. Своего монопольного доступа раннер не
берёт: блокировка, которая не пускает клиентов, отказывает и `ibcmd`. Окно между проверкой и
фазой данных закрывает платформа: `ibcmd` сам отказывает, если сеанс успел войти, и не
пускает новых клиентов на этой фазе. Решение об отказе раннер принимает по состоянию
блокировки, а не по тексту вывода платформы. Проверка — на POSIX; на Windows `ibcmd` для
`.dt` файловой базы отклоняется при выборе исполнителя и в превью, и в обычном
вызове, пока признак там не замерен. Ограничение хоста задаёт
`platform::ibcmd::dt_transfer_refusal`.

Командные строки и блокировки на Linux замерены на 8.5.1.1343
([замер](../../../references/1c/confirmed-runtime-measurements.md#ibcmd-dt-на-linux)).
