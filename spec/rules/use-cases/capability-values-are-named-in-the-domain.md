---
id: INV.USE-CASES.CAPABILITY-VALUES-ARE-NAMED-IN-THE-DOMAIN
check: [tests/architecture_guardrails.rs::capability_rows_are_written_in_one_place]
---

# Значения реализованности и улики называет только домен

Кто реализует операцию на цели данного вида, насколько и чем это доказано, говорит
`src/domain/capability.rs`, и только он называет в коде значения `Implementation` и
`Evidence` — ни записью, ни сравнением, ни сопоставлением с образцом их не называет никто
другой. Сценарии, проверка настроек и выбор исполнителя читают его ответ (`capabilities`,
`capability_of`, `default_chain`, `has_a_choice`): значение, названное мимо владельца, —
начало второй таблицы, которая расходится с ним молча и пропускает ключ `providers.*`,
который владелец отверг бы.
