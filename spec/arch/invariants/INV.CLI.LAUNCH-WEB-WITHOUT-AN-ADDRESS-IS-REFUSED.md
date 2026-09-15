---
id: INV.CLI.LAUNCH-WEB-WITHOUT-AN-ADDRESS-IS-REFUSED
status: active
governs: product
decision: DEC.2026-09-14.LAUNCH-OPENS-THE-PUBLISHED-BASE
check: tests/cli_launch.rs::launch_web_without_a_declared_address_is_refused_with_the_reason
scope: [cli]
---

# Открытие базы без объявленного адреса отклоняется

Без клиентского адреса команда отказывает и называет, что адрес появляется после публикации или задаётся вручную.
