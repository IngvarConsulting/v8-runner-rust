---
id: INV.CLI.A-FAILED-SNAPSHOT-NAMES-THE-RECIPE
check:
  - tests/cli_infobase_copy.rs::a_failed_snapshot_of_a_file_base_names_the_recipe
  - tests/cli_infobase_copy.rs::a_failed_snapshot_of_a_cluster_base_names_the_maintenance_window
---

# Неудавшийся снимок источника называет рецепт

Если `infobase create --from` не снял образ с источника, отказ называет, как освободить
источник: у файловой базы — закрыть Конфигуратор и клиенты копии-владельца из метки, у базы в
кластере — окно обслуживания `sessions deny` и `sessions terminate`, а после снимка
`sessions allow`. Рецепт называется при всякой неудаче снимка: причину по выводу
Конфигуратора раннер не угадывает (`INV.PLATFORM.PROSE-DEBT-ONLY-SHRINKS`). Сеансы раннер
сам не завершает; новой базы и брошенного образа неудача не оставляет.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
