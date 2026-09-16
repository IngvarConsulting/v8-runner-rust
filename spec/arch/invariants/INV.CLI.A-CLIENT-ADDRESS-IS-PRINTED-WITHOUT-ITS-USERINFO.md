---
id: INV.CLI.A-CLIENT-ADDRESS-IS-PRINTED-WITHOUT-ITS-USERINFO
status: active
governs: product
decision: DEC.2026-09-16.A-CLIENT-ADDRESS-IS-PRINTED-WITHOUT-ITS-USERINFO
check: tests/cli_launch.rs::a_client_address_is_reported_without_its_userinfo_password
scope: [cli, wire]
---

# Показанный клиентский адрес не несёт пароля

В `data.url`, `data.plan.args`, `message` и текстовом выводе пароль из userinfo заменён
на `***`; имя пользователя остаётся. В argv процесса идёт настоящий адрес. Журнал и текст
ошибок процесса — вне области: там действует своя политика.
