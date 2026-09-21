---
id: DEC.2026-09-21.TARGET-KIND-IS-ANSWERED-BY-THREE-QUESTIONS
status: active
governs: product
realized: [tests/cli_agent_standalone.rs::a_direct_gate_address_next_to_the_standalone_section_is_accepted_but_not_used_yet, tests/cli_agent_standalone.rs::a_file_address_next_to_the_standalone_section_is_refused, tests/cli_infobases.rs::a_connection_without_a_supported_shape_is_refused_as_neither_file_nor_server, tests/cli_publish.rs::a_web_connection_string_is_refused_as_an_administrative_channel]
supersedes: [DEC.2026-09-14.TARGET-KIND-IS-DECLARED-NOT-PARSED]
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
администрировать; отказ называет, что поставить вместо неё. Сверх сайта: третий ответ
получает только строка в форме серверного адреса, которую платформа принимает
(`Srvr=…;Ref=…` с непустыми частями); строка без `File=` и без такой формы — отказ
валидации с именем ожидаемой формы, а не кластер. Вид объявляется, а не выводится: по
одной строке подключения кластер от автономного сервера не отличить.

**Почему.** Прежнее правило «две декларации сразу — ошибка» считало `Srvr=` декларацией
кластера. С прямым шлюзом (`DEC.2026-09-21.A-STANDALONE-TARGET-HAS-TWO-GATES`) та же
строка ведёт и к автономному серверу, поэтому вид решает секция, а строка — только
адрес.

**Заменяет при реализации.** `DEC.2026-09-14.TARGET-KIND-IS-DECLARED-NOT-PARSED`. Его
правило `INV.CONFIG.TARGET-DECLARATIONS-ARE-EXCLUSIVE` переходит сюда: часть про `ws=…`
остаётся и расширяется до всякой строки без принимаемой формы, часть про две декларации
уступает порядку вопросов.

Источник: [`deployments.html#d-which`](../../../docs/site/deployments.html#d-which),
[`architecture.html#targets`](../../../docs/site/architecture.html#targets).
