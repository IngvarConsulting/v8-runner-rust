---
id: INV.CLI.EXPORT-GRAMMAR-IS-FIXED
status: active
governs: product
decision: DEC.2026-09-02.EXPORT-INTENTS-ARE-TYPED-SEPARATELY
check: tests/cli_help.rs::infobase_configuration_export_help_fixes_the_exact_grammar
scope: [cli, docs]
---

# Грамматика экспорта конфигурации закреплена

Справка команды содержит точную грамматику: состояние, предмет и требуемое расширение файла.
