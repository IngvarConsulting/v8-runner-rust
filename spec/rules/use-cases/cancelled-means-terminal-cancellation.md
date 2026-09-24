---
id: INV.USE-CASES.CANCELLED-MEANS-TERMINAL-CANCELLATION
check: [src/use_cases/interruption.rs::command_interruption_status_preserves_terminal_state]
---

# Статус отмены означает состоявшуюся отмену

Статус отмены ставится только при фактической терминальной отмене; успешное завершение после сигнала остаётся успехом с предупреждением об отложенном прерывании.
