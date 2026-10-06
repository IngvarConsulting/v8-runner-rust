---
id: INV.CLI.PULL-ALL-REFUSES-A-LISTED-NAME-THAT-IS-NOT-AN-IDENTIFIER
check:
  - src/platform/extension_inventory.rs::the_designer_name_list_is_read_line_by_line_and_fail_closed
  - tests/cli_pull_all.rs::a_listed_name_that_is_not_an_identifier_is_refused_before_any_dump
  - tests/cli_dump_agent.rs::pull_all_through_the_agent_refuses_a_name_that_is_not_an_identifier
---

# Имя из списка базы, не являющееся идентификатором, — отказ

Имя из списка расширений, которое не является идентификатором 1С, — неверный вывод
инструмента, а не набор: оно стало бы доводом платформы и каталогом. `pull --all` отказывает
до первой выгрузки и проектный файл не трогает — у любого исполнителя.
