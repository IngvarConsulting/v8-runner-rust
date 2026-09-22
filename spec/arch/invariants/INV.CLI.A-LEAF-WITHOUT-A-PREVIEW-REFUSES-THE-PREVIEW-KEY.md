---
id: INV.CLI.A-LEAF-WITHOUT-A-PREVIEW-REFUSES-THE-PREVIEW-KEY
status: active
governs: product
decision: DEC.2026-09-22.PREVIEW-IS-A-GLOBAL-KEY
check: tests/cli_global_flags.rs::a_leaf_without_a_preview_refuses_the_key_and_names_itself
scope: [cli]
---

# Лист без превью отвергает ключ превью

Команда, у которой превью нет, получив `--dry-run`, отказывает и называет себя и причину, а не выполняется молча.
