---
id: CTR.CLI.TEXT-OUTPUT
status: active
governs: product
version: 1
decision: DEC.2026-09-14.THE-HUMAN-SURFACE-IS-A-PINNED-SHAPE
artifact: docs/schemas/text-output.json
producer: src/output/text.rs
consumers: [cli, docs]
check: [src/output/text.rs::generated_text_output_grammar_is_current, tests/contract_text_output.rs::every_printed_line_is_a_declared_kind, tests/contract_text_output.rs::details_of_a_node_are_printed_in_the_declared_order]
scope: [cli, docs]
---

# Текстовый вывод для человека

Лента узлов: строка узла со знаком статуса, под ней подробности с отступом `│   `,
между узлами — пустая связка `│`. Отказ команды уходит в stderr строкой `ERROR: `.
Состав видов строк закрыт, и каждый описан образцом в артефакте.

Знак узла и есть статус: `●` сделано, `▲` сделано с предупреждениями, `✖` не сделано,
`◌` идёт, `○` пропущено.

Узлы бывают двух родов, и словарь знаков у них общий. **Узел этапа** открывает живой
прогресс, и знак его — состояние на момент открытия: `◌` у этапа, который начался и
ещё идёт. Исход этапа приходит ниже, подробностью со своим знаком. **Узел итога**
печатает раннер после команды, и его знак — вердикт: он же последняя строка ленты.

Знак итога выводится из содержимого узла, а не только из слова вызывающего:
предупреждение или всплывшая ошибка среди подробностей делают узел `▲`, даже если
команда дошла до конца. Так подпись узла не может разойтись с тем, что под ней.

Подробности одного узла печатаются одним порядком у всех команд: `ключ: значение`,
шаги со своим исходом, артефакты, улики, предупреждения, отказы. Порядок задан
перечнем `detail_kinds`; внутри вида сохраняется порядок, в котором строки собрала
команда. У живого прогресса порядка нет — он поток событий, и обещание порядка
относится к итоговому узлу.

Один и тот же текст не печатается дважды под разными пометками: остаётся самая точная
из них — та, что стоит в порядке последней.

Проза внутри строки не закреплена: форма отвечает за строение, а не за слова.

## Пример

```text
● init:
│   → infobase: create - would create a file infobase at 'build/ib' via /opt/1cv8/bin/1cv8
│
▲ Artifacts export completed with warnings
│   source-set: main
│   mode: cf
│   output: build/main.cf
│   [artifact] build/main.cf
│   [diagnostic] platform log -> build/logs/platform/make_0.log
│   [warning] would build ConfigurationCf into 'build/main.cf'; nothing published
│
✖ Artifact load failed
│   target: configuration
│   [error:artifact_load_failed] validation error: --path file does not exist: build/main.cf
```
