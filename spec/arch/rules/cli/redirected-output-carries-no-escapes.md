---
id: INV.CLI.REDIRECTED-OUTPUT-CARRIES-NO-ESCAPES
check: [tests/contract_text_output.rs::redirected_output_carries_no_escape_sequences]
---

# Перенаправленный вывод не несёт escape-последовательностей

Когда stdout не терминал, задана `NO_COLOR` или указан `--no-color`, ни stdout, ни
stderr не содержат ANSI-последовательностей.
