---
id: INV.PLATFORM.A-SERVICE-IS-TOLD-APART-BY-ITS-IDENTITY
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/536
---

# Свой процесс определяется по записи целиком

Процесс считается своим, когда совпали pid, время старта, хеш `machine_id` и имя образа из записи. Нет
`machine_id` — процесс чужой. Чужой процесс раннер не останавливает, а называет в ответе.
