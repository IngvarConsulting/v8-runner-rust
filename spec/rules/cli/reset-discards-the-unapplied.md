---
id: INV.CLI.RESET-DISCARDS-THE-UNAPPLIED
check:
  - tests/cli_reset.rs::reset_discards_a_push_without_apply_and_the_next_push_loads_it_again
  - tests/cli_reset.rs::reset_with_nothing_unapplied_rolls_nothing_back
  - tests/cli_reset.rs::reset_without_the_unapplied_state_is_refused_before_the_rollback
  - tests/cli_reset.rs::reset_without_a_set_rolls_back_only_the_main_configuration
  - tests/cli_reset.rs::reset_of_an_extension_that_is_not_installed_is_refused_before_work
  - tests/cli_reset.rs::reset_of_an_external_set_is_refused
  - tests/cli_reset.rs::reset_without_a_configuration_set_is_refused
  - tests/cli_reset.rs::a_failed_rollback_answers_a_platform_failure_and_the_next_push_loads_everything
  - tests/cli_reset.rs::an_edt_push_after_reset_loads_the_discarded_set_again
  - tests/cli_reset.rs::reset_creates_no_memory_of_the_base
  - tests/cli_reset.rs::reset_leaves_the_memory_of_another_pair
  - tests/cli_reset.rs::reset_preview_dispatches_nothing
  - tests/cli_reset.rs::providers_reset_assigns_the_executor_of_the_reset
  - tests/cli_agent_standalone.rs::a_standalone_reset_without_the_direct_gate_is_refused_before_any_session
  - tests/architecture_guardrails.rs::the_rollback_act_has_one_owner
  - tests/cli_reset.rs::reset_rewrites_the_record_with_the_tool_that_made_it
  - tests/cli_reset.rs::reset_into_a_base_that_moved_keeps_the_record_and_the_next_push_is_refused
  - tests/cli_reset.rs::reset_keeps_a_record_made_before_a_failed_load
  - tests/cli_reset.rs::reset_without_an_answer_erases_the_record
  - tests/cli_reset.rs::reset_erases_a_record_made_by_the_agent
---

# `reset` отбрасывает непринятое, не трогая базу данных

Команда возвращает основную конфигурацию к конфигурации базы данных и саму базу данных не
трогает — это обратный ход применения, ремонтный шаг. У расширения своя пара: без набора
команда откатывает основную конфигурацию, расширение — только названным набором; в проекте
без набора основной конфигурации (у него одни внешние наборы) `reset` отказывает до запуска
платформы.

Своё и чужое непринятое команда не различает: решение о сбросе принимает человек. До отката
она узнаёт, есть ли непринятое, тем же признаком, что `status --deep`: нет — отката нет и
ответ это называет; признак не получен — отказ до отката. Перед откатом память исходников
набора заменяется пустой, поэтому следующая отправка грузит отброшенное заново; новой
памяти о базе команда не создаёт. Исполнители — Конфигуратор и `ibcmd`; в наборе
SSH-шлюза отката нет.

Запись журнала поколений набора читает инструмент, который её сделал, до отката и после
него. Поколение до отката равно записи — запись переписывается ответом после отката с
прежним `after` и без пометки «не применено». База ушла от записи до отката или запись
сделана перед неудачной загрузкой — запись остаётся как есть, и следующая отправка, которая
грузит, отказывает `non_fast_forward`: откат не прячет чужую загрузку от этой проверки.
Ответа нет — запись стирается, и ответ это называет. Запись, сделанную агентом, `reset` не
читает — новой сессии агента ради токена не открывает, — а стирает и называет; следующая
отправка после этого грузит набор целиком без проверки, ушла ли база вперёд.

Решение владельца от 08.10.2026: сохранение записи при ушедшей базе и перед неудачной
загрузкой и стирание записи агента приняты.
