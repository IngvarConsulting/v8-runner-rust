---
id: INV.CLI.A-CLIENT-WAIT-NEEDS-A-PARAMETER-OR-AN-EPF
check:
  - tests/cli_launch.rs::thin_wait_without_c_or_execute_and_with_a_non_epf_execute_are_refused_before_spawn
---

# Ожидание клиента требует `--c` или обработку `.epf`

`launch thin --wait-for-exit` без `--c` и без `--execute`, как и с `--execute` не на
файл `.epf`, отказывает до запуска клиента: ждать нечего, а произвольный файл
обработкой не открывается. Клиент в таком случае не стартует.
