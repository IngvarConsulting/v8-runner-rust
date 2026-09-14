---
id: INV.USE-CASES.DEGRADATION-IS-VISIBLE
status: active
governs: product
decision: DEC.2026-04-20.DOUBT-TURNS-A-PARTIAL-LOAD-INTO-A-FULL-ONE
check: tests/cli_dump.rs::dump_ibcmd_partial_json_success_uses_degraded_fallback
scope: [use-cases, cli]
---

# Переход к более полному режиму назван в ответе

Если запрошенный режим недоступен и операция выполнена полнее, ответ называет и запрошенный режим, и причину деградации.
