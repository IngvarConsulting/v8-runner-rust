---
id: INV.USE-CASES.CANCELLED-MEANS-TERMINAL-CANCELLATION
status: active
governs: product
decision: DEC.2026-04-20.A-MUTATING-CRITICAL-PHASE-IS-NOT-HARD-KILLED
check: src/use_cases/interruption.rs::command_interruption_status_preserves_terminal_state
scope: [use-cases]
---

# Статус отмены означает состоявшуюся отмену

Статус отмены ставится только при фактической терминальной отмене; успешное завершение после сигнала остаётся успехом с предупреждением об отложенном прерывании.
