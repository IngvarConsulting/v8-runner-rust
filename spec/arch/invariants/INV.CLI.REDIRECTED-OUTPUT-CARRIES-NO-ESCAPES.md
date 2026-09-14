---
id: INV.CLI.REDIRECTED-OUTPUT-CARRIES-NO-ESCAPES
status: active
governs: product
decision: DEC.2026-09-14.THE-HUMAN-SURFACE-IS-A-PINNED-SHAPE
check: tests/contract_text_output.rs::redirected_output_carries_no_escape_sequences
scope: [cli]
---

# Перенаправленный вывод не несёт escape-последовательностей

Когда stdout не терминал, задана `NO_COLOR` или указан `--no-color`, ни stdout, ни
stderr не содержат ANSI-последовательностей.
