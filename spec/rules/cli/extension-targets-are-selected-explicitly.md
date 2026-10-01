---
id: INV.CLI.EXTENSION-TARGETS-ARE-SELECTED-EXPLICITLY
check:
  - tests/cli_extensions.rs::mixed_extension_selectors_preserve_exact_names_and_deduplicate_in_dispatch_order
  - tests/cli_extensions.rs::invalid_configured_selector_in_a_mixed_request_fails_before_clean_or_platform
---

# Цели правки свойств выбираются явно

`--name` называет набор-расширение, `--installed-name` — установленное расширение по платформенному имени; неизвестное `--name` отказывает, не ищет по базе и называет `--installed-name`. Все настроенные наборы берутся только тогда, когда не задан ни один селектор. Селекторы родительской команды с подкомандами не сочетаются, и негодное имя отвергается до замка, очистки и запуска платформы.
