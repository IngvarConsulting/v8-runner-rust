---
id: INV.USE-CASES.AN-APPLY-AFTER-A-FAILED-LOAD-IS-REFUSED
check:
  - tests/cli_apply.rs::an_apply_after_a_failed_load_is_refused
  - tests/cli_apply.rs::an_apply_after_a_failed_extension_load_is_refused_even_with_an_unchanged_generation
  - src/use_cases/apply.rs::an_apply_after_a_failed_load_without_an_answer_is_refused
  - src/use_cases/apply.rs::a_pending_cancellation_before_a_failed_load_refusal_answers_the_cancellation
---

# Применение после неудачной загрузки отказывает

Если запись набора помечена как сделанная перед неудачной загрузкой (`failed_build`),
`apply` отказывает `non_fast_forward` до запуска применения при любом поколении базы —
разошедшемся с записью, равном ей или без ответа инструмента записи — и первым выходом
называет повтор загрузки `push --force`: иначе структура базы данных перестроилась бы по
наполовину загруженной основной конфигурации. Равенство поколения неудачную загрузку не
исключает: Конфигуратор читает поколение применённого расширения, и загрузка его не
сдвигает. Отмена, пришедшая до отказа, отвечает отменой.
