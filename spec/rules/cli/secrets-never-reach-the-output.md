---
id: INV.CLI.SECRETS-NEVER-REACH-THE-OUTPUT
check:
  - src/platform/process.rs::render_command_masks_the_password_inside_a_connection_string
  - src/platform/secrets.rs::masks_a_password_quoted_around_a_semicolon
  - tests/cli_launch.rs::launch_failure_never_echoes_the_password_inside_the_connection_string
  - tests/cli_launch.rs::launch_dry_run_text_masks_credentials_and_says_nothing_was_dispatched
  - tests/cli_extensions.rs::extension_preview_never_echoes_the_infobase_password
---

# Пароль не появляется в выводе

Ни превью, ни квитанция, ни текст отказа, ни строка журнала не печатают пароль
информационной базы — ни отдельным значением ключа, ни внутри составленной строки
запуска, ни половиной закавыченного сегмента строки соединения. Это верно для всякого
показа составленных аргументов, а не только для того, на который смотрели.

Правило говорит о значении названного ключа. Пароль, приклеенный к ключу, которого
раннер не знает (`/Psecret`), отличим от постороннего ключа только по своему значению:
превью знает его из конфигурации и маскирует, а текст отказа строит платформенный слой,
который конфигурации не видит, и закрыть эту форму там нечем.

Маскируется показ, а не передача: argv дочернего процесса несёт пароль как прежде.
