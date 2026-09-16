---
id: DEC.2026-09-16.A-THIN-CLIENT-OPENS-EITHER-ADDRESS
status: active
governs: product
realized: tests/cli_launch.rs::a_thin_client_goes_through_the_web_address_when_asked
supersedes: []
superseded-by: null
establishes: [INV.CLI.A-STANDALONE-CLIENT-GOES-BY-THE-WEB-ADDRESS, INV.CLI.A-STANDALONE-CLIENT-IS-NOT-GIVEN-THE-GATE-CREDENTIALS, INV.CLI.A-NON-THIN-MODE-STILL-REFUSES-A-STANDALONE-TARGET, INV.CLI.VIA-IS-REJECTED-WHERE-THERE-IS-NO-CHOICE, CTR.WIRE.LAUNCH-DATA, CTR.MCP.PUBLISHED-TOOL-SURFACE]
changes: [CTR.WIRE.LAUNCH-DATA, CTR.MCP.PUBLISHED-TOOL-SURFACE]
---

# Тонкий клиент открывается любым из двух адресов цели

**Решение.** У цели два адреса — административный `infobase.connection` и клиентский
`infobase.web.url` ([`DEC.2026-09-14.TARGET-HAS-TWO-ADDRESSES`](2026-09-14-target-has-two-addresses.md)), —
и тонкий клиент открывается любым. Умолчание задаёт вид цели, объявленный, а не
разобранный: у автономного сервера административного адреса нет, поэтому его умолчание —
веб; у файловой и кластерной — строка подключения. Ключ `--via web|connection` умолчание
переопределяет и принимается везде, где клиентский режим тонкий, включая
`launch mcp --mode thin`. Где выбора нет — Конфигуратор, толстый, обычный, `launch web` —
ключ отвергается, а не принимается молча. Ответ несёт `via` у **каждого** режима: иначе
отсутствие поля пришлось бы толковать, а у формы `launch` уже есть прецедент обратного —
`provider_dispatched` присутствует в обоих режимах.

**Почему.** Автономный сервер раньше отказывал всем клиентским режимам и отправлял
человека в `launch web` — то есть в браузер. Но браузер не единственный клиент
опубликованной базы: тонкий клиент открывает её по ws-соединению и остаётся полноценным
клиентом 1С. Отказ был не про возможность, а про то, что раннер не умел передать адрес.

**Цена.** Веб-путь по-прежнему требует `1cv8c` на машине раннера — в отличие от
`launch web`, который открывает браузер и платформы не требует. Поэтому у пользователя
автономного сервера без платформы отказ меняется: вместо «use `launch web`» приходит отказ
поиска утилиты. Он называет опись установок
([`DEC.2026-09-16.A-MISSING-COMPONENT-IS-NAMED-WITH-ITS-INSTALLATIONS`](2026-09-16-a-missing-component-is-named-with-its-installations.md)),
так что человеку видно, чего именно не хватает.

**Реквизиты базы у автономной цели клиенту не передаются.** Там `infobase.user` и
`infobase.password` — учётные данные SSH-шлюза, а не базы, поэтому веб-путь против такой
цели отдаёт только адрес, без `/N` и `/P`. Реквизиты база спрашивает сама либо получает
через `tools.enterprise.additional-launch-keys`.

**Известный предел.** `reserved_launch_key` не резервирует ключей соединения, поэтому
пользовательский `/WS` или `/IBConnectionString` из `additional-launch-keys` допишется
после нашего, и порядок разрешает платформа. Это верно и до этого решения; резервирование
ключей соединения — отдельный предмет.

**Не затрагивает.** `ws=` внутри `infobase.connection` по-прежнему не принимается.
Доступность адреса не проверяется. Аутентификация на самом веб-сервере (`/WSN`, `/WSP`) —
не цель: раннер её не заводит и не хранит.
