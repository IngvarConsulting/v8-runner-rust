---
id: INV.CLI.CONCURRENT-PROCESSES-ARE-SERIALIZED
check: [tests/cli_infobase_cross_platform.rs::concurrent_native_cli_processes_observe_the_workspace_lock]
---

# Два процесса на одном каталоге выстраиваются в очередь

Одновременно запущенные процессы раннера наблюдают одну блокировку и не работают с каталогом параллельно.
