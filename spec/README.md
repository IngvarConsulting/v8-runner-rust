# Spec Guide

`spec/` хранит внутренний слой: согласованные гарантии продукта и его архитектурное
описание.

## Что где

- `arch/rules/`: правила продукта — согласованные гарантии, каждая со своей проверкой.
  Начинать с [`arch/README.md`](arch/README.md).
- `architecture/arc42/`: подробное описание архитектуры и набор рисков. Оно рассказывает,
  как устроено, и ничего не обещает.
- Открытые задачи ведутся в GitHub issues; сводный план по целевой модели — issue #233.

История правил — в Git. Замеры на живой платформе 1С лежат в
[`references/1c/`](../references/1c/README.md).

## Usage Rule

If a statement here conflicts with current code, CLI help, or the public docs layer, trust the
current code first and then update the active doc layer.
