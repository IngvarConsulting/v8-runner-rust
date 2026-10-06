---
id: INV.CLI.A-FILE-BASE-IS-LOCKED-FOR-THE-WHOLE-COMMAND
check:
  - tests/cli_infobase_lock.rs::a_second_command_on_a_held_base_is_refused_at_once_and_names_the_first
  - tests/cli_infobase_lock.rs::an_mcp_tool_on_a_held_base_is_refused_at_once
  - src/use_cases/infobase_lock.rs::a_held_base_refuses_a_second_command_and_names_the_first
---

# Файловая база занята на всё время команды

Команда, которая открывает файловую базу, держит замок рядом с её каталогом всё своё время
и этого замка не ждёт: занятая база — отказ, который называет, кто с ней сейчас работает.
Замок действует на одной машине.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
