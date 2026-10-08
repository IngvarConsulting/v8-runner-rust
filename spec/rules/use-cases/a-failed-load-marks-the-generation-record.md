---
id: INV.USE-CASES.A-FAILED-LOAD-MARKS-THE-GENERATION-RECORD
check:
  - tests/cli_push_generation.rs::after_a_failed_load_the_next_push_names_it_and_is_not_let_through
  - src/use_cases/build_project.rs::a_push_load_that_fails_after_a_deferred_cancellation_names_it
---

# Неудачная загрузка помечает запись поколения

После неудачной загрузки поколение не записывается, и шаг ответа это называет. Неудачной
считается загрузка в основную конфигурацию; отправку, у которой загрузка удалась, а
применение нет, описывает `INV.USE-CASES.A-PUSH-WHOSE-APPLY-FAILED-KEEPS-THE-LOAD`. Запись того
же инструмента помечается как сделанная перед неудачной загрузкой (`failed_build`). Если
поколение базы с ней совпадёт, следующая отправка идёт как обычно; если разойдётся — она
отказывает `non_fast_forward` и говорит, что базу изменила неудачная загрузка или другая
копия, и первым выходом называет повтор загрузки `push --force`, а не «база ушла вперёд с
прошлого обмена».
