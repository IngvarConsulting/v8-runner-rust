---
id: DEC.2026-09-14.TARGET-KIND-IS-DECLARED-NOT-PARSED
status: superseded
governs: product
realized: tests/cli_publish.rs::a_web_connection_string_is_refused_as_an_administrative_channel
supersedes: []
superseded-by: DEC.2026-09-21.TARGET-KIND-IS-ANSWERED-BY-THREE-QUESTIONS
establishes: [INV.CONFIG.TARGET-DECLARATIONS-ARE-EXCLUSIVE]
---

# Вид цели объявляется, а не выводится из строки подключения

**Решение.** Вид информационной базы объявляется в конфиге: `File=…` — файловая,
`Srvr=…;Ref=…` — кластер, секция `infobase.standalone` — автономный сервер. Две
декларации сразу — ошибка валидации: цель одна. Строка `ws=…` в
`infobase.connection` не принимается; валидация называет, что ставить вместо неё.

**Почему.** `ws=` — строка веб-подключения к опубликованной базе. За публикацией
может стоять файловая база, кластер или автономный сервер, который публикует себя
сам. Вывести из схемы URL, чем базу администрировать, нельзя — это догадка того же
сорта, что решение по тексту сообщения.

**Цена.** Пользователь, у которого в конфиге стоял `ws=`, обязан назвать
административный канал явно; молча угадать за него раннер отказывается.
