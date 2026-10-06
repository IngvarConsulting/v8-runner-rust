---
id: INV.CLI.AN-AD-HOC-CONNECTION-STRING-CARRIES-NO-CREDENTIALS
check:
  - tests/cli_infobases.rs::an_ad_hoc_connection_string_with_usr_is_refused
  - tests/cli_infobases.rs::an_ad_hoc_connection_string_with_pwd_is_refused
  - tests/cli_infobases.rs::an_ad_hoc_connection_string_with_wsn_is_refused
  - tests/cli_infobases.rs::an_ad_hoc_connection_string_with_wsp_is_refused
  - tests/cli_infobases.rs::an_ad_hoc_connection_string_with_wsppwd_is_refused
  - tests/cli_infobases.rs::an_ad_hoc_connection_string_with_password_is_refused
  - tests/cli_infobases.rs::an_ad_hoc_connection_string_with_the_n_key_is_refused
  - tests/cli_infobases.rs::an_ad_hoc_connection_string_with_the_p_key_is_refused
  - tests/cli_infobases.rs::an_ad_hoc_connection_string_part_without_equals_is_checked
  - tests/cli_infobases.rs::an_ad_hoc_connection_string_must_not_carry_credentials
  - src/platform/secrets.rs::a_connection_string_names_the_credential_key_as_written
  - src/platform/secrets.rs::a_part_without_equals_is_read_as_command_line_keys
---

# Строка соединения в `--infobase` не несёт учётных данных

База, выбранная строкой соединения в `--infobase`, учётных данных не несёт: их место —
местный слой, секция `infobases.<имя>`. Строка, в которой есть ключ учётных данных, —
отказ `invalid_argument` до запуска платформы. Учётные данные здесь — и секреты, и имена
пользователей: параметры `Usr`, `Pwd`, `Wsn`, `Wsp`, `Wsppwd`, `Password` и ключи
командной строки `/N`, `/P`, в том числе слитно со значением. Перечень ключей один на
продукт — тот, по которому вывод их маскирует; ключ, который вывод прячет, строка
`--infobase` не принимает.

Проверяется каждая часть строки, и часть без `=` не обрывает проверку остальных: она сама
читается как ключи командной строки. Отказ называет найденный ключ и местный слой, но не
повторяет ни строку, ни значение ключа.

Правило говорит о выборе базы. `init --infobase` базу объявляет и пишет строку в местный
слой — это другое место.
