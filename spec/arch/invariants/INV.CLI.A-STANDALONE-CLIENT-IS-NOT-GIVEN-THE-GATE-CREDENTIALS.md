---
id: INV.CLI.A-STANDALONE-CLIENT-IS-NOT-GIVEN-THE-GATE-CREDENTIALS
status: active
governs: product
decision: DEC.2026-09-16.A-THIN-CLIENT-OPENS-EITHER-ADDRESS
check: tests/cli_launch.rs::a_standalone_thin_client_carries_the_address_without_the_gate_credentials
scope: [cli]
---

# Клиенту автономной цели не отдают реквизиты шлюза

У автономной цели `infobase.user` и `infobase.password` — учётные данные SSH-шлюза, а не
базы. Поэтому веб-путь против неё несёт только адрес: ни `/N`, ни `/P` в командной строке
клиента не появляются. Реквизиты база спрашивает сама либо получает через
`tools.enterprise.additional-launch-keys`.
