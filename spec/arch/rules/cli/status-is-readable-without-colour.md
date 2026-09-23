---
id: INV.CLI.STATUS-IS-READABLE-WITHOUT-COLOUR
check:
  - tests/contract_text_output.rs::the_node_mark_agrees_with_the_exit_code
  - src/output/text.rs::a_node_mark_is_readable_without_colour
---

# Статус узла виден без цвета

У каждого состояния узла свой знак, и знаки различны между собой. Узел со знаком
отказа не появляется у команды, вышедшей с кодом 0.
