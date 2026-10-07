---
id: INV.CLI.SECRETS-NEVER-REACH-THE-OUTPUT
check:
  - src/platform/process.rs::render_command_masks_the_password_inside_a_connection_string
  - src/platform/secrets.rs::masks_a_password_quoted_around_a_semicolon
  - src/platform/secrets.rs::masks_a_connection_string_password_that_holds_a_space
  - src/platform/secrets.rs::a_connection_string_shown_as_a_value_hides_the_password_and_the_user
  - src/platform/secrets.rs::a_space_inside_a_value_does_not_split_its_masking
  - src/platform/secrets.rs::masks_the_password_of_a_connection_string_quoted_as_a_whole
  - tests/cli_launch.rs::launch_failure_never_echoes_the_password_inside_the_connection_string
  - tests/cli_launch.rs::launch_dry_run_text_masks_credentials_and_says_nothing_was_dispatched
  - tests/cli_extensions.rs::extension_preview_never_echoes_the_infobase_password
  - tests/cli_init.rs::server_infobase_create_never_echoes_the_connection_string_credentials
  - tests/cli_init.rs::a_failed_cluster_create_never_echoes_the_passwords
  - src/platform/secrets.rs::masks_the_dbms_and_cluster_passwords_of_a_creation_string
  - tests/cli_config_init.rs::init_in_a_new_worktree_redirects_the_copied_origin_and_keeps_it_as_upstream
---

# Пароль не появляется в выводе

Ни превью, ни квитанция, ни текст отказа, ни строка журнала не печатают пароль
информационной базы — ни отдельным значением ключа, ни внутри составленной строки
запуска, ни половиной закавыченного сегмента строки соединения. Это верно для всякого
показа составленных аргументов, а не только для того, на который смотрели. Так же скрыты
пароль СУБД и администратора кластера в строке `CREATEINFOBASE` (`DBPwd`, `SPwd`) — и в
показе аргументов, и в выводе платформы, который повторяет отказ создания базы в кластере.

Правило говорит о значении названного ключа. Пароль, приклеенный к ключу, которого
раннер не знает (`/Psecret`), отличим от постороннего ключа только по своему значению:
превью знает его из конфигурации и маскирует, а текст отказа строит платформенный слой,
который конфигурации не видит, и закрыть эту форму там нечем.

Маскируется показ, а не передача: argv дочернего процесса несёт пароль как прежде.
