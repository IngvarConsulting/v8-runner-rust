---
id: DEC.2026-09-14.PROVIDER-CHOSEN-PER-OPERATION
status: active
governs: product
realized: src/domain/capability.rs::an_experimental_provider_never_leads_a_default_chain
supersedes: []
superseded-by: null
establishes: [CTR.WIRE.INFOBASE-CONFIGURATION-EXPORT-DATA, CTR.WIRE.INFOBASE-DUMP-DATA, CTR.WIRE.INFOBASE-RESTORE-DATA]
changes: [CTR.WIRE.INFOBASE-CONFIGURATION-EXPORT-DATA, CTR.WIRE.INFOBASE-DUMP-DATA, CTR.WIRE.INFOBASE-RESTORE-DATA]
---

# Исполнителя выбирает пара «операция и цель»

**Решение.** Способ исполнения выбирается не одним ключом на проект, а по паре
«операция и вид информационной базы». Матрица `(операция, цель) → цепочка
провайдеров` живёт данными в `domain/capability.rs` и служит единственным
источником для валидации, выбора перед запуском и таблицы возможностей в
документации. Провайдеры закрытым набором: `designer`, `agent`, `ibcmd`,
`ibcmd-rs`, `webinst`. Онлайн- и офлайн-формы `ibcmd` — следствие вида цели, а не
отдельные провайдеры.

**Почему.** Ключ `builder` обещал глобальный выбор, которого нет: `convert` его не
использует, снимок информационной базы знает только Конфигуратор, экспорт
конфигурации давно трактует ключ как предпочтение и выбирает готового сам.
Агентский shell добавил исполнителя с собственным покрытием — у него нет проверки
конфигурации и сравнения, зато есть идентификатор поколения, — и склейка в один
ключ стала ложью.

**Цена.** Матрица становится публичным знанием: её содержимое видно в
документации и в квитанции, и её правка — изменение поведения, а не внутренняя
деталь.
