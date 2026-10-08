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
  - src/use_cases/reset.rs::without_a_configuration_set_the_refusal_names_the_extension_sets
  - tests/cli_reset.rs::a_failed_rollback_answers_a_platform_failure_and_the_next_push_loads_everything
  - tests/cli_reset.rs::an_edt_push_after_reset_loads_the_discarded_set_again
  - tests/cli_reset.rs::reset_creates_no_memory_of_the_base
  - tests/cli_reset.rs::reset_leaves_the_memory_of_another_pair
  - tests/cli_reset.rs::reset_preview_dispatches_nothing
  - tests/cli_reset.rs::providers_reset_assigns_the_executor_of_the_reset
  - tests/cli_agent_standalone.rs::a_standalone_reset_without_the_direct_gate_is_refused_before_any_session
  - tests/architecture_guardrails.rs::the_rollback_act_has_one_owner
---

# `reset` отбрасывает непринятое, не трогая базу данных

Команда возвращает основную конфигурацию к конфигурации базы данных и саму базу данных не
трогает — это обратный ход применения, ремонтный шаг. У расширения своя пара: без набора
команда откатывает основную конфигурацию, расширение — только названным набором; в проекте
без набора основной конфигурации команда без набора отказывает и называет наборы
расширений.

Своё и чужое непринятое команда не различает: решение о сбросе принимает человек. До отката
она узнаёт, есть ли непринятое, тем же признаком, что `status --deep`: нет — отката нет и
ответ это называет; признак не получен — отказ до отката. Перед откатом память исходников
набора заменяется пустой, поэтому следующая отправка грузит отброшенное заново; новой
памяти о базе команда не создаёт. Исполнители — Конфигуратор и `ibcmd`; в наборе
SSH-шлюза отката нет.
