---
id: INV.USE-CASES.AN-APPLY-AFTER-A-FAILED-LOAD-IS-REFUSED
check:
  - tests/cli_apply.rs::an_apply_after_a_failed_load_is_refused
  - src/use_cases/apply.rs::an_apply_after_a_failed_load_without_an_answer_is_refused
---

# Применение после неудачной загрузки отказывает

Если запись набора помечена как сделанная перед неудачной загрузкой (`failed_build`), а
поколение базы с ней разошлось, `apply` отказывает `non_fast_forward` до запуска
применения и первым выходом называет повтор загрузки `push --force`: иначе структура базы
данных перестроилась бы по наполовину загруженной основной конфигурации. Так же — когда
инструмент записи поколением не ответил: расхождения тогда не исключить.
