---
id: INV.PLATFORM.A-DECLARED-HOST-KEY-IS-THE-ONLY-ONE-ACCEPTED
check:
  - tests/cli_agent_standalone.rs::a_gate_that_presents_another_key_is_refused_by_name
  - tests/cli_agent_standalone.rs::sessions_reject_a_substituted_gateway_key_before_dispatch
  - tests/cli_agent_standalone.rs::sessions_report_a_gateway_that_refuses_the_scop_subsystem
---

# Объявленный ключ хоста — единственный принимаемый

Если ожидание по ключу хоста объявлено, сессия открывается только с ним. Другой ключ —
типизированный отказ, называющий ожидаемый и предъявленный отпечатки, а не сбой рукопожатия.
Отпечаток считается тем же алгоритмом, каким записан.

Объявленным считается то, что раннер смог прочесть: нечитаемый `host-key` ожидания не
даёт, и такая сессия идёт как несверяемая — вслух.

Для сеансов внешнего автономного сервера раннер проверяет `standalone.host-fingerprint`
в соединении, где будет открыта подсистема `scop`. При несовпадении аутентификация
пользователя ИБ и запрос к подсистеме не выполняются.
