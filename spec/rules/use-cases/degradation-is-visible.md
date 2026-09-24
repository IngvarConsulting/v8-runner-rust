---
id: INV.USE-CASES.DEGRADATION-IS-VISIBLE
check: [tests/cli_dump.rs::dump_ibcmd_partial_json_success_uses_degraded_fallback]
---

# Переход к более полному режиму назван в ответе

Если запрошенный режим недоступен и операция выполнена полнее, ответ называет и запрошенный режим, и причину деградации.
