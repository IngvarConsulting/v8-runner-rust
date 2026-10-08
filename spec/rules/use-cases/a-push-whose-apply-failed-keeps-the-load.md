---
id: INV.USE-CASES.A-PUSH-WHOSE-APPLY-FAILED-KEEPS-THE-LOAD
check:
  - tests/cli_apply.rs::a_push_whose_apply_failed_keeps_the_load
  - src/use_cases/build_project.rs::an_ibcmd_push_whose_apply_failed_keeps_the_load
  - src/use_cases/build_project.rs::an_edt_push_whose_apply_failed_keeps_the_generated_designer_snapshot
  - src/use_cases/build_project.rs::an_ibcmd_apply_that_fails_after_a_deferred_cancellation_names_it
---

# Отправка, у которой не удалось только применение, сохраняет загрузку

Если загрузка в основную конфигурацию удалась, а применение к конфигурации базы данных нет
(чаще всего из-за открытого сеанса), отправка запоминает загруженное как у `push
--no-apply`: память исходников фиксируется, поколение записывается с пометкой «загружено, не
применено» (`INV.USE-CASES.A-LOAD-WITHOUT-APPLY-IS-REMEMBERED-AS-UNAPPLIED`), а ответ
называет отказ применения и выход `apply`. Запись не помечается как неудачная загрузка.
