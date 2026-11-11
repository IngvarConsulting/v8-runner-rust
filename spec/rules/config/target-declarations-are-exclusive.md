---
id: INV.CONFIG.TARGET-DECLARATIONS-ARE-EXCLUSIVE
check:
  - tests/cli_agent_standalone.rs::a_direct_gate_address_next_to_the_ssh_gate_puts_the_designer_first
  - tests/cli_agent_standalone.rs::a_file_address_next_to_the_standalone_section_is_refused
  - tests/cli_infobases.rs::a_connection_without_a_supported_shape_is_refused_as_neither_file_nor_server
  - tests/cli_publish.rs::a_web_connection_string_is_refused_as_an_administrative_channel
  - tests/cli_infobases.rs::a_connection_with_a_control_character_is_refused_without_its_value
  - src/config/validate.rs::a_connection_with_a_control_character_is_refused_without_its_value
  - src/config/loader.rs::load_config_refuses_a_connection_with_a_control_character
---

# Вид цели объявлен ровно один раз

Вид цели отвечает на три вопроса по порядку: секция `standalone` — автономный сервер, иначе
`File=` в строке — файловая база, иначе — кластер. Рядом с секцией `standalone` строка — адрес
прямого шлюза серверной формы или пусто, и вида цели она не меняет; `File=` рядом с ней —
отказ. Строка веб-подключения `ws=…` в качестве административной не принимается, и отказ
называет, что поставить вместо неё. Строка без `File=` и без серверной формы, которую
платформа примет, — отказ с ожидаемой формой.

Строка соединения — одна строка печатных символов. Управляющий символ (ниже U+0020: `\r`,
`\n`, `\0` и прочие) в `connection` секции базы, в том числе на краю строки, или внутри строки
`--infobase`, чьи края обрезаются как пробельные, — отказ до запуска платформы: `invalid_argument`, род `validation`, выход 2.
Отказ называет ключ и код символа, но не значение: в строке бывают реквизиты.

Кластер под управлением раннера объявляет `cluster.managed` без строки: `INV.CONFIG.A-MANAGED-CLUSTER-DECLARES-NO-CONNECTION`.
