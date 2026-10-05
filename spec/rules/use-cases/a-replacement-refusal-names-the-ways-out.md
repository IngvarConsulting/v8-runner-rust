---
id: INV.USE-CASES.A-REPLACEMENT-REFUSAL-NAMES-THE-WAYS-OUT
check:
  - src/use_cases/destruction_guard.rs::the_refusal_names_the_ways_out_the_caller_has
  - src/use_cases/dump_config.rs::an_edt_extension_refusal_does_not_offer_a_bare_pull_force
  - src/use_cases/dump_config.rs::an_mcp_extension_refusal_names_the_exact_pull_for_the_same_source_set
  - src/use_cases/dump_config.rs::a_caller_without_a_consent_key_is_not_told_to_force
  - tests/cli_bootstrap.rs::a_clone_refusal_does_not_offer_force
  - tests/cli_convert.rs::a_convert_refusal_does_not_offer_a_truncated_command
---

# Отказ сторожа замены называет выходы, которые есть у вызывающего

Отказ заменить каталог с невосстановимой работой называет выходы: сохранить работу в
системе контроля версий (закоммитить или спрятать) и повторить — или заменить каталог с
её потерей.

Совет повторяет исходную цель. Готовой команды, урезанной до имени команды, отказ не
предлагает: без набора, каталога вывода и прочих аргументов исходного вызова буквальный
повтор бьёт в другой каталог. В командной строке совет — та же команда, повторённая с
добавленным ключом согласия. У MCP, где ключа нет, совет — точная команда строки для той
же цели: у `dump_config` — `v8-runner pull <SET> --force` с именем разрешённого набора.
Если собрать команду точно нельзя, совет называет команду строки для той же цели, не
подставляя урезанную.

Ключ согласия отказ предлагает только тому вызывающему, у которого он есть и действует.
Вызывающему без такого ключа (`clone`) отказ оставляет один выход — сохранить работу.
