---
id: INV.USE-CASES.A-CLUSTER-DESIGNER-GENERATION-IS-READ-LIKE-A-FILE-ONE
check:
  - tests/cli_status.rs::status_deep_reads_designer_generation_on_server_targets
  - src/platform/generation.rs::the_token_is_the_last_non_empty_line_of_forty_hex_digits
  - src/platform/generation.rs::an_unrecognized_answer_is_no_answer
---

# Ответ Конфигуратора о поколении у базы в кластере разбирается как у файловой

При работе с Конфигуратором раннер читает `/GetConfigGenerationID` у базы в кластере
и по прямому шлюзу автономного сервера так же, как у файловой базы. Нераспознанный
ответ считается отсутствием токена. Сравнение серверных токенов ограничено инструментом
записи. [Замер #184](https://github.com/IngvarConsulting/v8-runner-rust/pull/751)
