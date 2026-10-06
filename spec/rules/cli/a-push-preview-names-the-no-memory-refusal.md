---
id: INV.CLI.A-PUSH-PREVIEW-NAMES-THE-NO-MEMORY-REFUSAL
check:
  - tests/cli_push_generation.rs::a_preview_of_a_push_without_memory_names_the_refusal
---

# Превью `push` называет отказ без памяти

Превью `push` без памяти о базе отказывает так же, как отказал бы прогон
(`INV.USE-CASES.A-PUSH-WITHOUT-MEMORY-OF-THE-BASE-IS-REFUSED`), и платформу для этого не
запускает. Следов оно не оставляет и тут.

Решение владельца от 06.10.2026.
