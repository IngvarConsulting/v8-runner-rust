---
id: INV.CLI.A-FAILED-SNAPSHOT-NAMES-THE-RECIPE
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/330
---

# Неудавшийся снимок источника называет рецепт

Если `infobase create --from` не снял образ с источника, отказ называет, как освободить
источник: у файловой базы — закрыть Конфигуратор и клиенты копии-владельца, у базы в
кластере — окно обслуживания `sessions deny` и `sessions terminate`, а после снимка
`sessions allow`. Сеансы раннер сам не завершает.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
