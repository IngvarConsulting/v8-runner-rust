---
id: INV.USE-CASES.A-PUSH-WHOSE-APPLY-FAILED-KEEPS-THE-LOAD
check:
  - tests/cli_apply.rs::a_push_whose_apply_failed_keeps_the_load
  - src/use_cases/build_project.rs::an_ibcmd_push_whose_apply_failed_keeps_the_load
  - src/use_cases/build_project.rs::an_edt_push_whose_apply_failed_keeps_the_generated_designer_snapshot
  - src/use_cases/build_project.rs::an_ibcmd_apply_that_fails_after_a_deferred_cancellation_names_it
  - src/use_cases/build_project.rs::execute_ibcmd_build_honors_interruption_before_apply_safe_point
---

# Отправка, у которой не удалось только применение, сохраняет загрузку

Если загрузка в основную конфигурацию удалась, а применение к конфигурации базы данных нет
(чаще всего из-за открытого сеанса), отправка запоминает загруженное как у `push
--no-apply`: поколение записывается с пометкой «загружено, не применено»
(`INV.USE-CASES.A-LOAD-WITHOUT-APPLY-IS-REMEMBERED-AS-UNAPPLIED`), и только когда эта
запись легла в журнал, фиксируется память исходников; иначе память не фиксируется, и
следующая отправка загрузит и применит набор снова. Ответ называет отказ применения и выход
`apply`. Запись не помечается как неудачная загрузка.
