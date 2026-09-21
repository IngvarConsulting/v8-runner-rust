---
id: DEC.2026-09-21.INIT-DECLARES-AND-CLONE-PULLS
status: planned
governs: product
realized: null
supersedes: []
superseded-by: null
establishes: []
changes: [CTR.WIRE.CONFIG-INIT-DATA, CTR.WIRE.BOOTSTRAP-DATA]
---

# `init` объявляет проект, `clone` его выгружает

**Решение.** `init` заводит проект здесь: находит наборы исходников, пишет
`v8project.yaml` и местный слой, наружу не выходит. По умолчанию он объявляет `origin`
файловой базой внутри рабочего каталога (`File=build/ib`); `init --infobase <строка>`
записывает названный адрес в `origin` и базу не трогает. `clone --from <строка>` — это
`init --infobase` и `pull` в пустом каталоге одной командой: привязывает каталог к базе
и выгружает её; непустой каталог — отказ (сверх сайта: там сказано только «в пустом
каталоге»). Базу не создаёт ни одна из двух — это `infobase create`
(`DEC.2026-09-21.INFOBASE-CREATE-FOLLOWS-THE-TARGET-KIND`, он же преемник прежнего
`init`).

**Почему.** «Есть исходники, базы нет» и «есть база, исходников нет» — два входа в
проект, и у гита для них два слова. Прежний `bootstrap` делал второе под именем,
которого гит не знает, а `config init` — первое под двумя словами.

**Цена.** Форма ответа `clone` иная, чем у `bootstrap`: шаги привязки и выгрузки, а не
флаг `dumped`.

Источник: [`cli.html#map`](../../../docs/site/cli.html#map),
[`usecases.html`](../../../docs/site/usecases.html).
