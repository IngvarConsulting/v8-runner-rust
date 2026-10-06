---
id: INV.USE-CASES.CONFIGURATION-PACKAGES-ARE-WALKED-IN-THE-INVENTORY-ORDER
check:
  - src/use_cases/source_inventory.rs::configuration_packages_are_walked_in_one_order
  - tests/architecture_guardrails.rs::the_order_of_source_sets_is_decided_in_one_place
  - tests/cli_pull_all.rs::an_extension_without_a_set_is_declared_and_pulled
---

# Пакеты конфигурации обходятся порядком инвентаря наборов

`pull --all` обходит пакеты конфигурации проекта через
`SourceSetInventory::configuration_packages`: основная конфигурация, затем наборы расширений
в порядке объявления в `v8project.yaml`. Наборы внешних файлов пакетов конфигурации не
называют и в этот обход не входят. Порядок наборов по назначению решает
`source_inventory::ordered_by_purpose`; свой порядок, собранный корзинами по назначению, в
другом модуле не заводится. Страж ловит раскладку по корзинам (`SourceSetPurpose::… =>
список.push(…)`); свой `sort_by_key` по назначению в другом модуле он не видит.

Объявляемые наборы `pull --all` выгружает после наборов проекта и дописывает в конец
`source-set:`, поэтому следующий обход идёт тем же порядком.
