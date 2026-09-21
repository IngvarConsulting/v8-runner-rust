---
id: DEC.2026-09-21.DEFAULT-CHAINS-FOLLOW-THE-TARGET-KIND
status: planned
governs: product
realized: null
supersedes: []
superseded-by: null
establishes: [CTR.WIRE.INFOBASE-DUMP-DATA, CTR.WIRE.INFOBASE-RESTORE-DATA]
---

# Цепочки умолчаний строятся по виду цели

**Решение.** Исполнителей семь, набор закрыт: `designer`, `agent` (агент Конфигуратора
или SSH-шлюз `ibsrv`), `ibcmd`, `ibcmd-rs`, `webinst`, `rac`, `edt-cli`. Матрица
`(операция, вид цели) → цепочка` — по строкам таблицы сайта, столбцы: файловая · кластер
· автономный сервер.

Обмен `push`, `apply`, `pull`, `download` — `agent, designer, ibcmd` · `agent, designer`
· `designer, agent`. `upload` и снимок `infobase dump·restore` — `agent, designer` ·
`agent, designer` · `designer`. `reset` — `designer, ibcmd` · `designer` · `designer`.
`diff` — `ibcmd, designer` · `designer` · `designer, agent`. `extensions` —
`ibcmd, agent` · `agent, designer` · `agent, designer`. `infobase create` —
`ibcmd, designer` · `designer, rac` · нет. `make` — `ibcmd, ibcmd-rs, designer` для
всех; `convert` — `edt-cli, ibcmd, ibcmd-rs`; `check` — `designer`, для EDT `edt-cli`.
`sessions` — нет · `rac` · `ibcmd`. `publish` — `webinst` · `webinst` · нет. Строки
`test` и `launch` таблицы описывают клиент 1С и в матрицу не входят.

Доступность операции зависит только от вида цели. Матрица остаётся единственным
источником для валидации, выбора перед запуском, таблицы сайта и `docs/CAPABILITIES.md`;
расхождения подбора сайта с таблицей решены в пользу таблицы.

**Почему.** Агент держит одну сессию на команду и отдаёт поколение за секунду там, где
отдельный процесс Конфигуратора стоит четыре и больше; в обмене с базой он первый.
`ibcmd` не регистрирует базу в кластере и к базе под кластером не применяется — в
столбце кластера его нет. У автономного сервера полный набор есть только у Конфигуратора
по прямому шлюзу — он первый.

**Цена.** Умолчание `agent` требует локальной платформы у файловой и кластерной цели.

**Заменяет при реализации.** `DEC.2026-09-14.PROVIDER-CHOSEN-PER-OPERATION`. Задача #222
велела оставить его в силе, но перечень исполнителей закрыт в его тексте, а текст не
правят; преемник сохраняет матрицу как единственный источник и меняет только перечень.
`DEC.2026-09-14.PROVIDER-DEFAULTS-LIVE-IN-CODE` остаётся. Контракты
`CTR.WIRE.INFOBASE-DUMP-DATA` и `CTR.WIRE.INFOBASE-RESTORE-DATA` переходят сюда. Третий,
`CTR.WIRE.INFOBASE-CONFIGURATION-EXPORT-DATA`, раньше замены вывести нельзя: символ
держит действующий владелец, а правило выводит из обращения только преемник, — поэтому в
`DOWNLOAD-DATA` его выводит эта же замена; до неё форму `download` несёт прежний символ
(`DEC.2026-09-21.A-PACKAGE-IS-UPLOADED-AND-DOWNLOADED` называет его в `changes`).

Источник: [`architecture.html#d-ops`](../../../docs/site/architecture.html#d-ops),
[`architecture.html#provider-choice`](../../../docs/site/architecture.html#provider-choice),
[`index.html`](../../../docs/site/index.html).
