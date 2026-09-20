---
id: INV.PLATFORM.A-DECLARED-HOST-KEY-IS-THE-ONLY-ONE-ACCEPTED
status: active
governs: product
decision: DEC.2026-09-17.A-HOST-KEY-IS-CHECKED-AGAINST-WHAT-WAS-DECLARED
check: tests/cli_agent_standalone.rs::a_gate_that_presents_another_key_is_refused_by_name
scope: [platform]
---

# Объявленный ключ хоста — единственный принимаемый

Если ожидание по ключу хоста объявлено, сессия открывается только с ним. Другой ключ —
типизированный отказ, называющий ожидаемый и предъявленный отпечатки, а не сбой рукопожатия.
Отпечаток считается тем же алгоритмом, каким записан.

Объявленным считается то, что раннер смог прочесть: нечитаемый `host-key` ожидания не
даёт, и такая сессия идёт как несверяемая — вслух.
