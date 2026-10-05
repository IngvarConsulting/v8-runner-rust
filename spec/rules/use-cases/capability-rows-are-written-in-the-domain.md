---
id: INV.USE-CASES.CAPABILITY-ROWS-ARE-WRITTEN-IN-THE-DOMAIN
check: [tests/architecture_guardrails.rs::capability_rows_are_written_in_one_place]
---

# Строки матрицы исполнителей пишутся только в домене

Кто реализует операцию на цели данного вида, насколько и чем это доказано, говорит
`src/domain/capability.rs`. Сценарии, проверка настроек и выбор исполнителя читают его
ответ и своих таблиц реализованности или улики не заводят: таблица мимо владельца
расходится с ним молча и пропускает ключ `providers.*`, который владелец отверг бы.
