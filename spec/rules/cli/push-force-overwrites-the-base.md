---
id: INV.CLI.PUSH-FORCE-OVERWRITES-THE-BASE
check:
  - tests/cli_push_generation.rs::a_push_force_loads_without_memory_and_remembers_the_generation
  - tests/cli_push_generation.rs::a_push_into_a_base_that_moved_ahead_is_refused_before_the_load
---

# `push --force` перезаписывает базу

`push --force` грузит каждый выбранный набор целиком, без проверки памяти о базе и её
поколения: конфигурация в базе заменяется каталогом, и сделанное в ней теряется. Проверку
владельца он не обходит. Инструментов MCP с таким ключом нет: отказ, выданный инструменту,
называет команду командной строки.

Источник: [`cli.html`](../../../docs/site/cli.html).
