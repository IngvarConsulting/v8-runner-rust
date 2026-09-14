---
id: INV.CONFIG.TYPE-COMES-FROM-MARKER-CONTENT
status: active
governs: product
decision: DEC.2026-04-20.SOURCE-SET-TYPE-IS-DETECTED-FROM-MARKER-CONTENT
check: tests/cli_config_init.rs::config_init_detects_edt_extension_without_base_project_and_warns
scope: [config]
---

# Тип набора берётся из содержимого, а несоответствие названо

Определение типа опирается на содержимое маркер-файлов; неполный маркер даёт предупреждение с причиной, а не молчаливую догадку.
