---
id: INV.CLI.STATUS-DEEP-NAMES-THE-UNAPPLIED-AND-THE-DYNAMIC-UPDATE
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/412
---

# `status --deep` называет непринятое и динамическое обновление

`status --deep` говорит, есть ли в базе непринятое — основная конфигурация отличается от
конфигурации базы данных, — и обновлена ли база динамически. Признак берётся из
структурного ответа платформы, а не из её прозы; пока такой ответ не замерен, полей в форме
`CTR.WIRE.STATUS-DATA` нет.

Источник: [`sources.html`](../../../docs/site/sources.html), раздел о состояниях внутри базы;
[`cli.html#map`](../../../docs/site/cli.html#map).
