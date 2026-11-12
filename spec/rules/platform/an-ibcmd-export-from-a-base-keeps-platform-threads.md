---
id: INV.PLATFORM.AN-IBCMD-EXPORT-FROM-A-BASE-KEEPS-PLATFORM-THREADS
check:
  - src/platform/ibcmd.rs::only_the_package_export_is_single_threaded
  - src/platform/ibcmd.rs::config_export_full_builds_expected_args
  - src/platform/ibcmd.rs::config_export_for_changes_builds_expected_args
---

# Выгрузка `ibcmd` из базы идёт в потоках платформы

`ibcmd config export` из базы — полная выгрузка, выгрузка по изменившемуся (`--base`) и
выгрузка расширения — не несёт ключа `--threads`: число потоков выбирает платформа. Один
поток заметно замедляет выгрузку крупной конфигурации, а сбоев многопоточной выгрузки из базы
замеры не дали ([замер](../../../references/1c/confirmed-runtime-measurements.md), раздел о
потоках выгрузки `ibcmd`). Один поток остаётся только у разбора пакета `export --file`
(`INV.USE-CASES.IBCMD-EXPORTS-A-PACKAGE-IN-A-THROWAWAY-BASE`).
