---
id: INV.CLI.STATUS-DEEP-NAMES-THE-UNAPPLIED
check:
  - tests/cli_status.rs::status_deep_names_the_unapplied
  - tests/cli_status.rs::status_deep_without_a_platform_answers_null_with_a_reason
---

# `status --deep` называет непринятое

`status --deep` говорит, есть ли в базе непринятое: основная конфигурация отличается от
конфигурации базы данных. Признак берётся из структурного ответа платформы, а не из её прозы:
побайтное неравенство основной конфигурации и конфигурации базы данных, сохранённых в файл
([замер](../../../references/1c/confirmed-runtime-measurements.md), раздел о признаках
непринятого). Признак называет поле `base.unapplied` формы `CTR.WIRE.STATUS-DATA`: у набора
расширения — для самого расширения. Спрашивает тот же исполнитель, что поколение (исполнитель
`push`): Конфигуратор — `/DumpCfg` и `/DumpDBCfg`, `ibcmd` — `config save` и `config save --db`.
Сохранение у агента не замерено: у него `unapplied` — `null` с причиной.

Состояние «обновлено динамически» `status --deep` не называет: на 8.3.27 Конфигуратор и `ibcmd`
не отдают его снаружи сеанса, а внутри сеанса оно видно только тому, кто был подключён до
обновления (тот же замер,
решение владельца от 07.10.2026).

Источник: [`sources.html`](../../../docs/site/sources.html), раздел о состояниях внутри базы;
[`cli.html#map`](../../../docs/site/cli.html#map).
