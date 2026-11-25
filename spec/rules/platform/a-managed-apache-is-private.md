---
id: INV.PLATFORM.A-MANAGED-APACHE-IS-PRIVATE
check:
  - src/platform/apache.rs::a_private_configuration_names_only_its_loopback_port_and_files
  - src/platform/service.rs::a_linux_listener_is_identified_by_its_socket
  - src/use_cases/managed_stand/web.rs::a_taken_port_moves_apache_and_the_answer_names_the_previous_one
  - src/use_cases/managed_stand/web.rs::an_apache_holding_the_private_config_is_refused_with_its_pid
  - src/use_cases/managed_stand/web.rs::the_runner_never_runs_the_system_apache_controls
  - src/use_cases/managed_stand/web.rs::a_crashed_start_leaves_an_apache_that_status_and_stop_take_over
  - src/use_cases/managed_stand/web.rs::only_webinst_is_elevated_and_a_refusing_sudo_names_the_sudoers_rule
  - src/use_cases/managed_stand/web.rs::publish_as_root_is_refused_before_anything_runs
  - src/use_cases/managed_stand/web.rs::reclaim_rewrites_a_foreign_file_as_the_runner_with_its_content
  - src/use_cases/managed_stand/web.rs::reclaim_refuses_a_foreign_directory_and_names_the_command_to_remove_it
  - src/use_cases/managed_stand/web.rs::reclaim_without_owners_changes_nothing
  - src/use_cases/managed_stand/web.rs::reclaim_leaves_symbolic_links_alone
  - src/use_cases/managed_stand/web.rs::files_webinst_leaves_as_root_are_taken_back_after_success_and_failure
  - src/use_cases/managed_stand/web.rs::webinst_comes_from_the_platform_search_not_from_the_cluster_record
---

# Частный Apache запускается отдельно

Конфиг, журнал и PID-файл лежат под `localPath`. Порт вне динамического диапазона.
Apache слушает loopback, его модуль взят из выпуска кластера или выпуска `webinst` для файловой ИБ. Занятый порт ведёт
к выбору другого порта, занятый конфиг — к отказу с PID. Системные команды
управления Apache не используются, а `tools.apache.path` на них — отказ: раннер
останавливает и перезапускает свой экземпляр по записи службы и идентичности
процесса, а экземпляр упавшего запуска узнаёт по своему PID-файлу, порту и конфигу.

Права root на Linux получает только `webinst` — через `sudo -n`, разрешённый правилом
sudoers для этой программы; от записи от root такое правило пользователя не ограждает. Остальное `publish` делает от обычного пользователя, а от
root отказывает. Путь `webinst` даёт поиск утилит, а не запись кластера под
`localPath`, и выпуск найденного совпадает с выпуском работающего кластера. Файлы,
которые `webinst` записал под каталогом Apache, раннер переписывает от своего имени и
после удачи, и после отката: под `localPath` не остаётся файлов root. Каталог root раннеру
не переписать — отказ называет его и команду, которой его убрать.
