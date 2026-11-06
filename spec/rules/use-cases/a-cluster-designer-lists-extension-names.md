---
id: INV.USE-CASES.A-CLUSTER-DESIGNER-LISTS-EXTENSION-NAMES
check:
  - tests/provider_matrix.rs::a_cluster_base_refuses_ibcmd_for_every_operation_it_left
  - tests/cli_extensions.rs::cluster_designer_lists_only_names_with_batch_argv
  - tests/cli_extensions.rs::cluster_designer_refuses_extension_operations_other_than_list
---

# Конфигуратор перечисляет имена расширений кластера

В строке кластера для `extensions` агент первый, пакетный Конфигуратор второй. У Конфигуратора
подтверждён `list` через `/DumpDBCfgList -AllExtensions`. Выбор Конфигуратора для остальных
действий даёт `capability_unavailable` до запуска платформы и называет агента.

**Reintroduction guard.** Строка матрицы описывает семейство целиком. Ограничение по действию
принадлежит `extension_inventory::select_executor`. Тест отказа проверяет все остальные входы
семейства и фиксирует отсутствие запуска платформы.
