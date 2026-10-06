---
id: INV.CLI.STATUS-DEEP-NAMES-THE-OWNING-COPY
check:
  - tests/cli_status.rs::status_deep_names_the_owning_copy_and_writes_nothing
---

# `status --deep` называет копии-владельцы

У файловой базы `status --deep` называет рабочие копии, которые её держат, по метке
владельца, и отмечает среди них ту, из которой спрашивают. Ни в метку, ни в память под
`workPath` он ничего не пишет; пишутся только журналы — Конфигуратора и сессии агента.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies),
[`cli.html#map`](../../../docs/site/cli.html#map).
