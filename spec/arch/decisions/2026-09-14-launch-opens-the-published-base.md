---
id: DEC.2026-09-14.LAUNCH-OPENS-THE-PUBLISHED-BASE
status: active
governs: product
realized: tests/cli_launch.rs::launch_web_dry_run_names_the_opener_and_the_address
supersedes: []
superseded-by: null
establishes: [INV.CLI.LAUNCH-WEB-WITHOUT-AN-ADDRESS-IS-REFUSED, CTR.WIRE.LAUNCH-DATA]
changes: [CTR.WIRE.LAUNCH-DATA]
---

# `launch web` открывает опубликованную базу

**Решение.** `v8-runner launch web` открывает `infobase.web.url` в браузере. Без
объявленного адреса — типизированный отказ, называющий, что адрес появляется после
публикации или задаётся вручную. Тонкий клиент по веб-подключению остаётся обычным
`launch thin` с тем же адресом.

**Почему.** Веб-клиент — это адрес в браузере, а не утилита платформы. Отдельный
вид у `launch` нужен, чтобы у публикации был наблюдаемый результат: команда,
которая этот результат открывает.

**Не затрагивает.** Проверку доступности адреса: раннер не пингует публикацию и не
чинит её.
