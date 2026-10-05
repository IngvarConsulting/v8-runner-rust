---
id: INV.CONFIG.A-RESOLVED-CONFIG-PATH-IS-BUILT-FROM-ITS-COMPONENTS
check:
  - src/support/path.rs::resolve_from_drops_current_dir_components_and_keeps_parent_dir
  - src/support/path.rs::resolve_from_builds_native_windows_paths
  - tests/cli_bootstrap.rs::clone_with_a_dotted_source_dir_hands_the_platform_a_clean_path
---

# Разрешённый путь конфига собран из своих компонентов

Путь конфига, разрешённый от каталога основного `v8project.yaml`, не несёт внутренних `.`,
повторных разделителей и приставки `\\?\`, а разделители в нём родные для ОС — в том числе
у пути, записанного с `/` на Windows. `..` остаётся как есть: лексическое сворачивание
прошло бы мимо символьной ссылки.
