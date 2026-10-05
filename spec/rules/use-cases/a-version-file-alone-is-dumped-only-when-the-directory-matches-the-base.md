---
id: INV.USE-CASES.A-VERSION-FILE-ALONE-IS-DUMPED-ONLY-WHEN-THE-DIRECTORY-MATCHES-THE-BASE
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/214
---

# Один файл версий выгружается, только когда каталог совпадает с базой

Платформа умеет выгрузить один файл версий, не трогая остального (`-configDumpInfoOnly`).
Такой файл описывает базу такой, какая она сейчас, и заявляет, что все её объекты уже лежат
в каталоге. Раннер выгружает его только тогда, когда совпадение каталога и базы доказано:
сразу после удачного `push` или полного `pull`, пока поколение конфигурации не изменилось.
Так он восстанавливает потерянный файл версий без полной выгрузки. В остальных случаях
выгрузка полная.

Источник: [`platform.html#t21`](../../../docs/site/platform.html#t21),
[`sources.html#runner`](../../../docs/site/sources.html#runner).
