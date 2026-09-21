---
id: DEC.2026-09-21.A-STANDALONE-TARGET-HAS-TWO-GATES
status: planned
governs: product
realized: null
supersedes: []
superseded-by: null
establishes: [INV.CONFIG.A-STANDALONE-TARGET-ACCEPTS-EITHER-GATE-KEY, INV.PLATFORM.AN-AGENT-FOR-A-FILE-OR-CLUSTER-TARGET-NEEDS-THE-LOCAL-PLATFORM, INV.CLI.A-STANDALONE-CLIENT-IS-NOT-GIVEN-THE-GATE-CREDENTIALS, INV.CLI.A-NON-THIN-MODE-STILL-REFUSES-A-STANDALONE-TARGET]
changes: [INV.CLI.A-STANDALONE-CLIENT-IS-NOT-GIVEN-THE-GATE-CREDENTIALS, INV.CLI.A-NON-THIN-MODE-STILL-REFUSES-A-STANDALONE-TARGET]
---

# У автономной цели два шлюза

**Решение.** У автономного сервера `ibsrv` две точки входа, и объявляются они двумя
ключами секции базы. Прямой шлюз — `connection: Srvr=<хост[:порт]>;Ref=<имя>` (имя равно
`--name` сервера, порт — предмет замера #178); он принимает Конфигуратор и тонкий
клиент, файлы остаются у раннера, набор операций полный. SSH-шлюз — `standalone.gate`:
исполняет урезанный агентский набор сам, через канал обмена. Достаточно любого из двух
ключей, второй расширяет набор. Толстый клиент и обычное приложение прямой шлюз не
пускает — отказ типизированный. `user` и `password` — пользователь базы; по нему же
пускает SSH-шлюз, поэтому клиент по прямому шлюзу получает их, как у любой цели. Раннер
сервер не поднимает и не ищет — он ваш. Как секция `standalone` соотносится со строкой
`Srvr=` — `DEC.2026-09-21.TARGET-KIND-IS-ANSWERED-BY-THREE-QUESTIONS`; порядок
исполнителей — `DEC.2026-09-21.DEFAULT-CHAINS-FOLLOW-THE-TARGET-KIND`.

**Почему.** Клиенты подключаются к автономному серверу как к кластеру
([`platform.html#t64`](../../../docs/site/platform.html#t64)). Пока раннер знал только
SSH-шлюз, у автономной цели не было ни отката, ни сравнения, ни проверок, ни снимка —
всё это есть у Конфигуратора по прямому шлюзу.

**Цена.** Платформа на машине раннера нужна прямому шлюзу; без неё остаётся SSH-путь.

**Заменяет при реализации.**
`DEC.2026-09-14.ONLY-A-STANDALONE-SERVER-ANSWERS-WITHOUT-BEING-STARTED`. Его правило
`INV.PLATFORM.AN-AGENT-FOR-A-FILE-OR-CLUSTER-TARGET-NEEDS-THE-LOCAL-PLATFORM` остаётся
верным и переходит сюда. Два правила `DEC.2026-09-16.A-THIN-CLIENT-OPENS-EITHER-ADDRESS`
об автономной цели — клиенту не отдают реквизиты шлюза, нетонкий режим отказывает —
становятся неверными уже здесь: при реализации они переписываются под это решение, а их
владелец заменяется позже
(`DEC.2026-09-21.LAUNCH-THIN-TAKES-THE-DECLARED-CONNECTION-STRING`, #208).

Источник: [`architecture.html#targets`](../../../docs/site/architecture.html#targets),
[`deployments.html#d-standalone-local`](../../../docs/site/deployments.html#d-standalone-local).
