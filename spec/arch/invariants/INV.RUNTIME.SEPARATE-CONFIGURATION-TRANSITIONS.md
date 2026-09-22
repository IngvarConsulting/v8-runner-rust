---
id: INV.RUNTIME.SEPARATE-CONFIGURATION-TRANSITIONS
status: active
governs: product
decision: DEC.2026-09-22.COMPATIBLE-CONFIGURATION-TRANSITIONS
check: [tests/cli_load.rs::load_no_apply_stops_after_cf_and_cfe_artifact_load, tests/cli_configuration_transition.rs::transitions_use_exact_designer_operation_and_extension_for_file_and_cluster, tests/cli_configuration_transition.rs::busy_workspace_blocks_execution_but_not_read_only_preview]
scope: [cli, runtime]
---

# Загрузка, применение и откат имеют разные эффекты

load --no-apply останавливается после загрузки CF/CFE; обычный load сохраняет
совместимость. Apply и reset не загружают исходники или пакет. Явная цель
расширения не заменяется основной конфигурацией при ошибке. Reset требует
force; preview новых команд не пишет файлы и не берёт lock.
