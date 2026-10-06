---
id: INV.USE-CASES.WITHOUT-A-GENERATION-ANSWER-A-NO-MEMORY-REFUSAL-OFFERS-PULL-FORCE
check:
  - tests/cli_push_generation.rs::without_a_generation_answer_a_no_memory_refusal_offers_pull_force
---

# Без ответа о поколении отказ без памяти советует полную выгрузку

Выгрузка поверх каталога записывает память о базе, только если инструмент ответил
поколением. Если инструмент выгрузки этой цели заведомо не отвечает, отказ `no_memory`
советует вместо `pull <SET>` полную выгрузку `pull <SET> --force` — следующим шагом и
текстом — и предупреждает, что она заменит каталог и потеряет незакоммиченное в нём.

Заведомо не отвечает Конфигуратор у базы в кластере, пока формат его ответа не замерен
(`INV.USE-CASES.A-CLUSTER-DESIGNER-GENERATION-IS-READ-LIKE-A-FILE-ONE`, [#184](https://github.com/IngvarConsulting/v8-runner-rust/issues/184)):
с замером это исключение снимается.

Решение владельца от 06.10.2026.
