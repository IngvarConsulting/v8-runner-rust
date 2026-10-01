---
id: INV.CONFIG.TYPE-COMES-FROM-MARKER-CONTENT
check:
  - tests/cli_config_init.rs::config_init_detects_edt_extension_without_base_project_and_warns
  - src/use_cases/config_init.rs::detects_edt_external_aggregate_root_from_direct_child_projects
  - src/use_cases/config_init.rs::ambiguous_edt_external_root_is_not_detected
  - src/use_cases/config_init.rs::external_only_autodiscovery_fails_without_phantom_configuration
---

# Тип набора берётся из содержимого, а несоответствие названо

Определение типа опирается на содержимое маркер-файлов; неполный маркер даёт предупреждение
с причиной, а не молчаливую догадку. Корнем внешних обработок EDT становится каталог, в
прямых подкаталогах которого лежат опознанные проекты внешних обработок; если среди них
есть неопознанный проект, набора этот корень не даёт. Без набора конфигурации
автоопределение отказывает и не выдумывает его.
