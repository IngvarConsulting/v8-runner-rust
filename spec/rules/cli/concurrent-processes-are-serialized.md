---
id: INV.CLI.CONCURRENT-PROCESSES-ARE-SERIALIZED
check:
  - tests/cli_infobase_cross_platform.rs::concurrent_native_cli_processes_observe_the_workspace_lock
  - src/use_cases/workspace_lock.rs::conflicts_use_canonical_workspace_path_and_sidecar_metadata
  - src/use_cases/workspace_lock.rs::stale_sidecar_metadata_falls_back_to_generic_busy_message
  - tests/cli_infobase.rs::ready_provider_reports_workspace_busy_without_dispatch
---

# Два процесса на одном каталоге не работают одновременно

Одновременно запущенные процессы раннера наблюдают одну блокировку и не работают с каталогом
параллельно. Блокировка стоит на каноническом пути `workPath`: путь через символическую
ссылку упирается в ту же блокировку, а отказ называет канонический путь и, когда запись о
владельце относится к этой блокировке, того, кто её держит. Второй не ждёт: он отказывает
сразу, исполнителя не запускает и цели не создаёт.
