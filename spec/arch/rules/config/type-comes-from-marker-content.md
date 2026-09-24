---
id: INV.CONFIG.TYPE-COMES-FROM-MARKER-CONTENT
check: [tests/cli_config_init.rs::config_init_detects_edt_extension_without_base_project_and_warns]
---

# Тип набора берётся из содержимого, а несоответствие названо

Определение типа опирается на содержимое маркер-файлов; неполный маркер даёт предупреждение с причиной, а не молчаливую догадку.
