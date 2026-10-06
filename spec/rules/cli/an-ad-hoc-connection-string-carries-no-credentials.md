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
  - tests/cli_infobases.rs::an_ad_hoc_connection_string_with_a_quoted_key_is_refused
  - tests/cli_infobases.rs::an_ad_hoc_connection_string_with_any_masked_key_is_refused
  - tests/cli_infobases.rs::an_ad_hoc_connection_string_with_a_password_in_the_web_address_is_refused
  - tests/cli_infobases.rs::an_ad_hoc_connection_string_must_not_carry_credentials
  - src/platform/secrets.rs::every_key_the_output_masks_is_refused
  - src/platform/secrets.rs::a_connection_string_names_the_credential_key_as_written
  - src/platform/secrets.rs::a_part_without_equals_is_read_as_command_line_keys
  - src/platform/secrets.rs::a_raw_connection_is_checked_by_the_tokens_the_platform_gets
  - src/platform/secrets.rs::a_glued_key_is_named_without_its_value
  - src/platform/secrets.rs::a_password_in_the_address_userinfo_is_refused_by_its_key
---

# Строка соединения в `--infobase` не несёт учётных данных

База, выбранная строкой соединения в `--infobase`, учётных данных не несёт: их место —
местный слой, секция `infobases.<имя>`. Строка, которая несёт хоть что-то из того, что
вывод маскирует, — отказ `invalid_argument` до запуска платформы. Перечень один на
продукт — тот, по которому вывод прячет и секреты, и имена пользователей: параметры
`Usr`, `Pwd`, `Wsn`, `Wsp`, `Wsppwd`, `Password`, `WspUser`, ключи командной строки
`/N`, `/P`, `/WSN`, `/WSP`, `/UC`, `/AccessToken` и прочие из него, в том числе слитно со
значением, а также имя или пароль в адресе (`ws=http://alice:…@host`, `/WS http://…`).

Проверяется каждая часть строки, и часть без `=` не обрывает проверку остальных: она сама
читается как ключи командной строки. Сырая форма проверяется теми токенами, которые
получит платформа: кавычки сняты, закавыченный путь с пробелом — одно значение. Отказ
называет найденный ключ (у слитной формы — кратчайший ключ перечня, с которого начинается
слово) и местный слой, но не повторяет ни строку, ни значение.

Правило говорит о выборе базы. `init --infobase` базу объявляет и пишет строку в местный
слой — это другое место.
