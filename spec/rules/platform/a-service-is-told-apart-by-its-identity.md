---
id: INV.PLATFORM.A-SERVICE-IS-TOLD-APART-BY-ITS-IDENTITY
check:
  - src/platform/service_record.rs::a_process_is_own_only_when_every_part_of_the_identity_matches
  - src/platform/service_record.rs::a_machine_without_an_id_makes_every_process_foreign
  - src/use_cases/managed_stand/mod.rs::a_recycled_pid_is_not_taken_for_the_service
  - src/use_cases/managed_stand/mod.rs::a_leftover_pid_file_after_a_reboot_never_names_a_foreign_process
  - src/platform/service_record.rs::a_record_copied_to_another_place_names_a_foreign_process
  - src/use_cases/managed_stand/mod.rs::a_copied_local_directory_does_not_adopt_the_foreign_postgres
  - src/use_cases/managed_stand/mod.rs::a_shared_local_directory_leaves_the_stand_of_the_other_project_alone
---

# Свой процесс определяется по записи целиком

Процесс считается своим, когда совпали pid, время старта, хеш `machine_id` и имя образа из записи. Нет
`machine_id` — процесс чужой. Чужой процесс раннер не останавливает, а называет в ответе.

Запись службы стенда хранит ещё отпечатки канонического пути своего каталога и каталога проекта и сам каталог
проекта. Другой каталог — `.local` скопирован: запись устаревшая, а сервер, который она называет, чужой; раннер его не
останавливает и его остатки не снимает. Тот же каталог при другом проекте — `localPath` общий у нескольких проектов: запись
живая и чужая, команды стенда её не трогают и отказывают `workspace_busy`, называя проект, который её записал.
