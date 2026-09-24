---
id: INV.PLATFORM.LISTING-OUTPUT-IS-PARSED-FAIL-CLOSED
check:
  - tests/cli_extensions.rs::extensions_command_json_failure_without_payload_keeps_machine_readable_error
  - src/use_cases/extension_agent.rs::a_record_without_a_flag_is_refused_not_defaulted
---

# Перечень разбирается с отказом, а не с умолчанием

Запись перечня без обязательного поля и поле с ответом «да или нет», принявшее иное
значение, отклоняются как неверный вывод инструмента. Умолчание вместо отказа не
подставляется.

Неизвестное поле разбор не ломает: закрыт состав обязательных полей, а не всех.
