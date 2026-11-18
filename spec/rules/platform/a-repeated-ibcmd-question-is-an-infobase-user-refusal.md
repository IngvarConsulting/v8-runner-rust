---
id: INV.PLATFORM.A-REPEATED-IBCMD-QUESTION-IS-AN-INFOBASE-USER-REFUSAL
check:
  - src/platform/process/capture.rs::a_question_repeated_from_the_start_is_recognised_in_any_encoding
  - src/platform/process/capture.rs::the_oem_question_is_the_platform_text
  - src/platform/process/capture.rs::ordinary_output_is_not_a_question
  - src/platform/process/capture.rs::a_stream_without_a_slot_is_not_examined
  - src/platform/process.rs::a_process_repeating_a_question_on_closed_input_is_stopped
  - src/platform/process.rs::a_repeated_question_is_left_alone_without_the_policy_mark
  - src/platform/ibcmd.rs::every_ibcmd_run_stops_a_repeated_question_as_an_infobase_user_refusal
  - src/platform/ibcmd.rs::the_generation_question_carries_the_declared_credentials
  - tests/cli_ibcmd_user_question.rs::pull_force_refuses_an_infobase_that_asks_for_its_user
  - tests/cli_ibcmd_user_question.rs::push_force_refuses_an_infobase_that_asks_for_its_user
  - tests/architecture_guardrails.rs::process_output_has_one_bounded_reader_and_ibcmd_one_guarded_launch
---

# Вопрос `ibcmd` без ответа — отказ `infobase_user` за секунды

Ввод у `ibcmd` закрыт. Если у базы есть пользователи, а раннеру пользователь не дан,
`ibcmd` пишет пояснение и спрашивает «Имя пользователя: » снова и снова, не завершаясь.
Так было в замерах `dump`, `restore` и `config reset` (8.3.27) и `config generation-id`
(8.5.4.2028, #742) в `references/1c/confirmed-runtime-measurements.md`. Пароль он
спрашивает один раз и отказывает с кодом 255.

Каждый вызов `ibcmd`, обращающийся к базе, идёт через `IbcmdDsl::run_with` с политикой
`stopping_a_repeated_question`. Процесс снимается целиком, с группой, когда его поток с
самого начала — после пояснения не длиннее 4 КиБ — только повторяет один кусок не длиннее
512 байт, в котором есть не только пробелы, и так набрал 256 КиБ. Признак
структурный: язык и кодировка вопроса его не меняют, а проза инструмента ничего не решает
(`INV.PLATFORM.PROSE-DEBT-ONLY-SHRINKS`). Такой процесс ничего не сделал, только спрашивал,
поэтому его снимают при любом классе прерывания, в том числе `CriticalNonAbortable`. Признак
сильнее отложенного прерывания: ответ называет вопрос.

Ответ — `environment_unavailable` со слоем `infobase_user`
(`INV.WIRE.AN-ENVIRONMENT-REFUSAL-NAMES-ITS-LAYER`), выход 2. Текст называет базу, учётную
запись, повторённый вопрос как улику и что сделать: объявить `infobases.<имя>.user` и
`password` в `v8project.local.yaml`. С объявленными `user` и `password` вопроса нет: они
идут в каждый такой вызов, `config generation-id` тоже.

Признак ставит адаптер утилиты, а не исполнитель за всех: прочие утилиты исполнитель не
снимает. Признак видит только собираемый вывод (`Captured`), а не процессы с выводом в
никуда (`Managed*`). Вне правила — `ibcmd session list` автономного сервера
(`src/platform/ibsrv.rs`): он к базе не обращается, ограничен сроком `SESSIONS_TIMEOUT`, а
его сбой значит лишь «улик о сеансах нет».

Пределы. Живой прогон на 8.5.4.2028 с объявленным пользователем и без него — приёмка #742,
тесты его не заменяют. Вопрос, перед которым процесс вывел больше пояснения, под признак не
попадает.
