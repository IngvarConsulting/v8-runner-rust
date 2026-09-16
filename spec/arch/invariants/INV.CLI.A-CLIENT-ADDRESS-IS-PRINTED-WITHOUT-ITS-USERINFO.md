---
id: INV.CLI.A-CLIENT-ADDRESS-IS-PRINTED-WITHOUT-ITS-USERINFO
status: active
governs: product
decision: DEC.2026-09-16.A-CLIENT-ADDRESS-IS-PRINTED-WITHOUT-ITS-USERINFO
check: [tests/cli_launch.rs::a_client_address_is_reported_without_its_userinfo_password, tests/cli_launch.rs::a_real_web_launch_passes_the_unmasked_address_to_the_client, src/platform/enterprise.rs::mask_url_userinfo_hides_the_password_in_every_shape_of_address]
scope: [cli, wire]
---

# Показанный клиентский адрес не несёт пароля

В `data.url`, `data.plan.args`, `message` и текстовом выводе пароль из userinfo заменён
на `***`; имя пользователя остаётся. В argv процесса идёт настоящий адрес. Журнал и текст
ошибок процесса — вне области: там действует своя политика.
