---
id: INV.USE-CASES.EVERY-SET-IS-CHECKED-BEFORE-THE-FIRST-LOAD
check:
  - tests/cli_push_generation.rs::every_set_is_checked_before_the_first_load
  - tests/cli_build.rs::an_edt_set_with_a_skipped_export_is_checked_before_the_first_load
---

# Поколение всех наборов сверяется до первой загрузки

Когда в базу пойдут несколько наборов, поколение каждого сверяется до первой загрузки
команды: отказ `non_fast_forward` по набору не приходит после того, как наборы перед ним уже
загружены. В формате EDT сюда входит и набор, чей этап EDT пропущен, а копия Конфигуратора
изменилась и потому грузится. Чтения поколения те же, что сделала бы сверка перед каждой
загрузкой, — только раньше.
