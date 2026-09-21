---
id: DEC.2026-09-21.TARGET-KIND-IS-ANSWERED-BY-THREE-QUESTIONS
status: planned
governs: product
realized: null
supersedes: []
superseded-by: null
establishes: [INV.CONFIG.TARGET-DECLARATIONS-ARE-EXCLUSIVE]
changes: [INV.CONFIG.TARGET-DECLARATIONS-ARE-EXCLUSIVE]
---

# Вид цели отвечает на три вопроса по порядку

**Решение.** Вид информационной базы определяется тремя вопросами в этом порядке: есть
секция `standalone` — автономный сервер; иначе в строке подключения есть `File=` —
файловая база; иначе — кластер. Строка `Srvr=…;Ref=…` рядом с секцией `standalone` — не
вторая декларация вида, а адрес прямого шлюза; `File=` рядом с ней — ошибка: файлового
адреса у автономного сервера нет. Строка веб-подключения `ws=…` в `connection`
по-прежнему не принимается: она называет клиентский адрес и не говорит, чем базу
администрировать; отказ называет, что поставить вместо неё. Вид объявляется, а не
выводится: по одной строке подключения кластер от автономного сервера не отличить.

**Почему.** Прежнее правило «две декларации сразу — ошибка» считало `Srvr=` декларацией
кластера. С прямым шлюзом (`DEC.2026-09-21.A-STANDALONE-TARGET-HAS-TWO-GATES`) та же
строка ведёт и к автономному серверу, поэтому вид решает секция, а строка — только
адрес.

**Заменяет при реализации.** `DEC.2026-09-14.TARGET-KIND-IS-DECLARED-NOT-PARSED`. Его
правило `INV.CONFIG.TARGET-DECLARATIONS-ARE-EXCLUSIVE` переходит сюда: часть про `ws=…`
остаётся, часть про две декларации уступает порядку вопросов.

Источник: [`deployments.html#d-which`](../../../docs/site/deployments.html#d-which),
[`architecture.html#targets`](../../../docs/site/architecture.html#targets).
