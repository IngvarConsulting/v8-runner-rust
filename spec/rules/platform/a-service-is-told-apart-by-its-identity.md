---
id: INV.PLATFORM.A-SERVICE-IS-TOLD-APART-BY-ITS-IDENTITY
check:
  - src/platform/service_record.rs::a_process_is_own_only_when_every_part_of_the_identity_matches
  - src/platform/service_record.rs::a_machine_without_an_id_makes_every_process_foreign
  - src/use_cases/managed_stand/mod.rs::a_recycled_pid_is_not_taken_for_the_service
  - src/use_cases/managed_stand/mod.rs::a_leftover_pid_file_after_a_reboot_never_names_a_foreign_process
---

# Свой процесс определяется по записи целиком

Процесс считается своим, когда совпали pid, время старта, хеш `machine_id` и имя образа из записи. Нет
`machine_id` — процесс чужой. Чужой процесс раннер не останавливает, а называет в ответе.
