---
id: INV.CLI.PUSH-FORCE-OVERWRITES-THE-BASE
check:
  - tests/cli_push_generation.rs::a_push_force_loads_without_memory_and_remembers_the_generation
  - tests/cli_push_generation.rs::a_push_into_a_base_that_moved_ahead_is_refused_before_the_load
  - tests/cli_push_generation.rs::a_full_push_into_a_base_that_moved_ahead_is_refused
  - tests/cli_infobase_owner.rs::a_connection_string_never_owns_and_warns_on_a_base_of_another_copy
  - tests/cli_push_generation.rs::a_full_push_with_memory_of_another_base_is_refused_as_no_memory
---

# `push --force` перезаписывает базу

`push --force` грузит каждый выбранный набор целиком, без проверки памяти о базе и её
поколения: конфигурация в базе заменяется каталогом, и сделанное в ней теряется. Это
единственный ключ, который обходит эти проверки: `--full` грузит целиком, но проверки
проходит. Проверку владельца `--force` не обходит. Инструментов MCP с таким ключом нет:
отказ `no_memory` или `non_fast_forward`, выданный инструменту, называет команду командной
строки `v8-runner push --force`.

Источник: [`cli.html`](../../../docs/site/cli.html).
