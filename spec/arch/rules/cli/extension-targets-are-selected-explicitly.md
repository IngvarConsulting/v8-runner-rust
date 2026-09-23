---
id: INV.CLI.EXTENSION-TARGETS-ARE-SELECTED-EXPLICITLY
check: [tests/cli_extensions.rs::mixed_extension_selectors_preserve_exact_names_and_deduplicate_in_dispatch_order]
---

# Цели правки свойств выбираются явно

`--name` называет набор-расширение, `--installed-name` — установленное расширение по платформенному имени; неизвестное `--name` отказывает и не ищет по базе. Все настроенные наборы берутся только тогда, когда не задан ни один селектор. Селекторы родительской команды с подкомандами не сочетаются, и негодное имя отвергается до замка, очистки и запуска платформы.
