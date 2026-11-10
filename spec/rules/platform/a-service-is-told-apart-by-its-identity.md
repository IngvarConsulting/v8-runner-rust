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
  - src/use_cases/managed_stand/mod.rs::a_stopped_stand_of_another_project_is_refused_and_its_data_is_kept
  - src/use_cases/managed_stand/mod.rs::after_a_reboot_the_stand_of_another_project_is_still_refused
  - src/platform/service_record.rs::the_owner_of_a_stand_outlives_its_record
  - src/use_cases/managed_stand/cluster.rs::the_cluster_and_its_base_refuse_the_stand_of_another_project
  - src/use_cases/managed_stand/standalone.rs::the_server_and_its_base_refuse_the_stand_of_another_project
  - src/config/loader.rs::a_working_command_names_the_project_owning_a_shared_stand
---

# Свой процесс определяется по записи целиком

Процесс считается своим, когда совпали pid, время старта, хеш `machine_id` и имя образа из записи. Нет
`machine_id` — процесс чужой. Чужой процесс раннер не останавливает, а называет в ответе.

Запись службы стенда хранит ещё отпечатки канонического пути своего каталога и каталога проекта и сам каталог
проекта. Другой каталог — `.local` скопирован: запись устаревшая, а сервер, который она называет, чужой; раннер его не
останавливает и его остатки не снимает.

Стенд базы принадлежит проекту, который первым завёл каталог его служб: метка
`<localPath>/infobases/<база>/managed/owner.json` хранит те же отпечатки и сам проект и, в отличие от записи о процессе,
переживает `stop` и перезагрузку. Метка того же каталога при другом проекте — `localPath` общий у нескольких проектов:
стенд чужой, работает он или остановлен. Команды стенда (`dbms`, `cluster`, `standalone`, `infobase create` базы стенда)
и рабочие команды над ним его не трогают и отказывают `workspace_busy`, называя проект-владелец; `status` и `platforms`
верхнего уровня показывают память, не трогая стенд. Метка из другого каталога — копия: она ничего не значит, и первая служба
стенда заменяет её своей.
