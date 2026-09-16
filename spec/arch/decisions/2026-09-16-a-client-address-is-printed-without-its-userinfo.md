---
id: DEC.2026-09-16.A-CLIENT-ADDRESS-IS-PRINTED-WITHOUT-ITS-USERINFO
status: active
governs: product
realized: tests/cli_launch.rs::a_client_address_is_reported_without_its_userinfo_password
supersedes: []
superseded-by: null
establishes: [INV.CLI.A-CLIENT-ADDRESS-IS-PRINTED-WITHOUT-ITS-USERINFO]
---

# Клиентский адрес печатается без пароля из userinfo

**Решение.** Там, где раннер показывает клиентский адрес — `data.url`, `data.plan.args`,
`message` и выведенный из них текстовый вывод, — пароль из userinfo заменяется на `***`.
Маскируется только то, что после `:`: `http://alice:s3cret@host/base` становится
`http://alice:***@host/base`. Имя пользователя остаётся — по нему адрес узнаётся.
В argv процесса идёт настоящий адрес: иначе клиент не подключится.

**Почему.** `infobase.web.url` не проходит валидации вовсе — `validate_web_publication`
выходит раньше, если `web.server` не задан, — поэтому userinfo в нём возможен, а
`mask_launch_args` знал только три способа спрятать секрет и адрес среди них не числился.
Отказывать на валидации было бы дороже: поле не проверяется сегодня, и отказ сломал бы
конфиги, которые работают.

**Область.** Только `data` команды и текстовый вывод. Журнал запуска процесса и текст
ошибок процесса идут через другой маскировщик — `render_command`, — и он по существующему
закреплённому решению оставляет видимой строку подключения вместе с `Pwd=`. Прятать
userinfo там, где виден `Pwd=`, непоследовательно: политика `render_command` — отдельный
предмет с отдельным решением, и `/WSP` относится к нему же.
