---
id: INV.CLI.REDIRECTED-OUTPUT-CARRIES-NO-ESCAPES
check: [tests/contract_text_output.rs::redirected_output_carries_no_escape_sequences]
---

# Перенаправленный вывод не несёт escape-последовательностей

Когда указан `--no-color` или задана `NO_COLOR`, ни stdout, ни stderr не содержат
ANSI-последовательностей. То же, когда stdout не терминал, — если не задана
`FORCE_COLOR`: её ставят те, кто перенаправляет вывод в средство, которое ANSI отрисует,
в первую очередь CI. Запрет сильнее разрешения, а разрешение сильнее догадки.
