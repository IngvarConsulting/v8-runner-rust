---
id: INV.CLI.CONCURRENT-PROCESSES-ARE-SERIALIZED
check: [tests/cli_infobase_cross_platform.rs::concurrent_native_cli_processes_observe_the_workspace_lock]
---

# Два процесса на одном каталоге не работают одновременно

Одновременно запущенные процессы раннера наблюдают одну блокировку и не работают с каталогом
параллельно. Второй не ждёт: он сразу отказывает с кодом `workspace_busy` на шаге
`workspace lock` и ничего не выгружает.
