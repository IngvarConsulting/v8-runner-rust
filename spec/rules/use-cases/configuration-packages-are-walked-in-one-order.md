---
id: INV.USE-CASES.CONFIGURATION-PACKAGES-ARE-WALKED-IN-ONE-ORDER
check:
  - src/use_cases/source_inventory.rs::configuration_packages_are_walked_in_one_order
  - tests/cli_pull_all.rs::an_extension_without_a_set_is_declared_and_pulled
---

# Пакеты конфигурации обходятся одним порядком

Команда, которая без набора идёт по всем пакетам конфигурации проекта, обходит их в одном
порядке: основная конфигурация, затем наборы расширений в порядке объявления в
`v8project.yaml`. Наборы внешних файлов пакетов конфигурации не называют и в этот обход не
входят. Порядок держит один владелец — `SourceSetInventory::configuration_packages`.

`pull --all` после наборов проекта выгружает объявляемые им наборы по имени и дописывает их
в конец `source-set:`, поэтому следующий обход идёт тем же порядком.
