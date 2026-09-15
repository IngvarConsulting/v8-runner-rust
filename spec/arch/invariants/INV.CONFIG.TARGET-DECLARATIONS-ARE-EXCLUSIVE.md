---
id: INV.CONFIG.TARGET-DECLARATIONS-ARE-EXCLUSIVE
status: active
governs: product
decision: DEC.2026-09-14.TARGET-KIND-IS-DECLARED-NOT-PARSED
check: tests/cli_publish.rs::a_web_connection_string_is_refused_as_an_administrative_channel
scope: [config]
---

# Вид цели объявлен ровно один раз

Две декларации цели сразу — ошибка; строка веб-подключения в качестве административной не принимается, и отказ называет, что поставить вместо неё.
