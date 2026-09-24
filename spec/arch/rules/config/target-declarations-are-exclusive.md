---
id: INV.CONFIG.TARGET-DECLARATIONS-ARE-EXCLUSIVE
check:
  - tests/cli_agent_standalone.rs::a_direct_gate_address_next_to_the_standalone_section_is_accepted_but_not_used_yet
  - tests/cli_agent_standalone.rs::a_file_address_next_to_the_standalone_section_is_refused
  - tests/cli_infobases.rs::a_connection_without_a_supported_shape_is_refused_as_neither_file_nor_server
  - tests/cli_publish.rs::a_web_connection_string_is_refused_as_an_administrative_channel
---

# Вид цели объявлен ровно один раз

Вид цели отвечает на три вопроса по порядку: секция `standalone` — автономный сервер, иначе
`File=` в строке — файловая база, иначе — кластер. Рядом с секцией `standalone` строка — адрес
прямого шлюза серверной формы или пусто; `File=` рядом с ней — отказ. Строка веб-подключения
`ws=…` в качестве административной не принимается, и отказ называет, что поставить вместо неё.
Строка без `File=` и без серверной формы, которую платформа примет, — отказ с ожидаемой формой.
