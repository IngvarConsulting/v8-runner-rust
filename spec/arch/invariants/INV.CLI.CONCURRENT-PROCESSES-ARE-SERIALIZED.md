---
id: INV.CLI.CONCURRENT-PROCESSES-ARE-SERIALIZED
status: active
governs: product
decision: DEC.2026-04-20.THE-OS-LOCK-IS-THE-TRUTH-THE-SIDECAR-IS-DIAGNOSTICS
check: tests/cli_infobase_cross_platform.rs::concurrent_native_cli_processes_observe_the_workspace_lock
scope: [cli]
---

# Два процесса на одном каталоге выстраиваются в очередь

Одновременно запущенные процессы раннера наблюдают одну блокировку и не работают с каталогом параллельно.
