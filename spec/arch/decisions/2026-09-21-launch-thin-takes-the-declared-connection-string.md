---
id: DEC.2026-09-21.LAUNCH-THIN-TAKES-THE-DECLARED-CONNECTION-STRING
status: planned
governs: product
realized: null
supersedes: []
superseded-by: null
establishes: [INV.CLI.VIA-IS-REJECTED-WHERE-THERE-IS-NO-CHOICE, CTR.WIRE.LAUNCH-DATA]
changes: [INV.CLI.A-STANDALONE-CLIENT-GOES-BY-THE-WEB-ADDRESS, CTR.WIRE.LAUNCH-DATA]
---

# `launch thin` берёт объявленную строку подключения

**Решение.** Правило адреса одно для всех видов цели: `launch thin` без ключа берёт
строку подключения, веб-адрес `web.url` — когда строка не объявлена. Ключ `--via
web|connection` переопределяет умолчание и принимается только там, где клиент тонкий,
включая `launch mcp --mode thin`; где выбора нет — Конфигуратор, толстый, обычный,
`launch web` — ключ отвергается. Ответ несёт `via` у каждого режима. Клиент тестов идёт
тем же правилом. У автономного сервера это значит: строка прямого шлюза объявлена —
тонкий клиент идёт по ней и получает учётные данные базы; не объявлена — идёт по
веб-адресу, который сервер отдаёт сам.

**Почему.** Прежнее умолчание «автономный сервер — веб» держалось на посылке, что
административного адреса у него нет и `user`/`password` — реквизиты шлюза. С прямым
шлюзом (`DEC.2026-09-21.A-STANDALONE-TARGET-HAS-TWO-GATES`) обе посылки перестают быть
верными, и особый случай исчезает.

**Заменяет при реализации.** `DEC.2026-09-16.A-THIN-CLIENT-OPENS-EITHER-ADDRESS`.
Принцип двух адресов, правило `INV.CLI.VIA-IS-REJECTED-WHERE-THERE-IS-NO-CHOICE` и
контракт `CTR.WIRE.LAUNCH-DATA` переходят сюда; правило
`INV.CLI.A-STANDALONE-CLIENT-GOES-BY-THE-WEB-ADDRESS` сужается до случая без объявленной
строки и переписывается под новым именем; `CTR.MCP.PUBLISHED-TOOL-SURFACE` уходит к
`DEC.2026-04-20.MCP-DOES-NOT-MIRROR-CLI` — владельцу неизменяемой поверхности. Замена
идёт после `DEC.2026-09-21.A-STANDALONE-TARGET-HAS-TWO-GATES` (#205 раньше #208): два
правила об автономной цели, которые то решение меняет, до тех пор держатся заменяемым
владельцем.

Источник: [`architecture.html#d-ops`](../../../docs/site/architecture.html#d-ops),
[`deployments.html#d-file-web`](../../../docs/site/deployments.html#d-file-web),
[`deployments.html#d-cluster-web`](../../../docs/site/deployments.html#d-cluster-web).
