---
id: INV.CLI.APPLY-SESSIONS-GOVERNS-FOREIGN-SESSIONS
check:
  - src/use_cases/apply/act.rs::a_server_apply_with_force_ends_the_sessions_and_names_them
  - src/use_cases/apply/act.rs::a_server_apply_without_force_keeps_the_platform_refusal
  - src/use_cases/apply/act.rs::a_forced_apply_nobody_held_terminates_nothing
  - src/use_cases/apply/act.rs::a_file_base_held_by_a_session_is_refused_with_its_list_even_with_force
  - src/use_cases/apply/act.rs::a_file_base_open_in_the_designer_is_refused_as_designer_busy
  - src/use_cases/apply/act.rs::an_agent_update_always_forbids_dynamic_update
  - src/use_cases/apply/sessions.rs::the_agent_is_held_by_sessions_only_by_its_error_type_or_its_question
  - src/use_cases/apply/sessions.rs::a_file_base_never_gets_the_key_that_ends_sessions
  - src/use_cases/apply/sessions.rs::a_relative_quoted_file_base_is_probed_where_the_platform_opens_it
  - src/platform/designer.rs::update_db_cfg_always_forbids_dynamic_update
  - src/platform/ibcmd.rs::config_apply_always_forbids_dynamic_update
  - src/platform/ibcmd.rs::config_apply_names_what_it_does_with_foreign_sessions
  - src/platform/session_list.rs::parses_the_measured_russian_list
  - src/platform/session_list.rs::parses_the_measured_english_list
  - src/platform/session_list.rs::an_unrecognised_line_drops_the_whole_list
  - src/platform/file_base_hold.rs::a_foreign_lock_names_its_message
  - tests/cli_apply_sessions.rs::a_file_base_held_by_a_session_is_refused_with_the_session_list
  - tests/cli_apply_sessions.rs::force_does_not_end_sessions_of_a_file_base
  - tests/cli_apply_sessions.rs::a_push_held_by_sessions_keeps_the_load_and_names_the_same_push
  - tests/cli_apply_sessions.rs::a_file_base_open_in_the_designer_refuses_the_load
  - tests/cli_apply_sessions.rs::force_on_a_cluster_ends_the_sessions_and_names_them
  - tests/cli_apply_sessions.rs::a_sessions_message_without_force_is_refused
  - tests/cli_apply_sessions.rs::push_without_apply_takes_no_sessions_key
---

# `--sessions` решает судьбу чужих сеансов

`push` и `apply` принимают `--sessions disable|force` и `--sessions-message <текст>`; без
ключа — `disable`. `make` ключа не принимает: база сборки своя. `push --no-apply` его тоже не
принимает: применения нет. У MCP параметра нет, и отказ приходит туда как `runtime_failure`.

**Признак.** Что базу держат, раннер решает по состоянию, а не по тексту платформы
(`INV.PLATFORM.PROSE-DEBT-ONLY-SHRINKS`): у агента — `error-type`
`InfoBaseExclusiveLockRequired` (или его вопрос «Отмена»/«Повторить», на который раннер отвечает
«Отмена»), у файловой базы — блокировка файлов: `1Cv8.1CD` держит любой сеанс, `1Cv8.cgr` —
Конфигуратор. Обновление с запретом динамического при любом сеансе файловой базы не проходит
(замер #211), поэтому отказ исполнителя при занятой базе — отказ из-за сеансов. У Конфигуратора и `ibcmd` сервера такого признака нет, и их
отказ остаётся отказом платформы. Текст отказа даёт только список сеансов и разбирается
целиком или никак (`INV.PLATFORM.LISTING-OUTPUT-IS-PARSED-FAIL-CLOSED`).

**Отказ `infobase_sessions`** (род `workspace`, выход 3): обновлению нужен монопольный
доступ, а базу держат сеансы. `error.sessions` называет базу (`base`, строка соединения без
пароля), сеансы (`list`: компьютер, код и название приложения, начало, пользователь и номер;
`null`, если список не получен или не разобран), `from_this_machine`, `terminable` и
блокировку начала сеансов `lock: {state: none|own|foreign|unknown, message}`. `next` — тот же
вызов: с `--sessions force`, если сеансы можно завершить, иначе без ключей — «закрыл,
повторить».

**Отказ `infobase_designer_busy`** (род `workspace`, выход 3): файловую базу держит чужой
Конфигуратор. `push` отказывает уже на загрузке. Поле `sessions` той же формы: `list: null` —
платформа не называет ни компьютер, ни пользователя, — `terminable: false`, `next` — тот же
вызов без ключей. Конфигуратор раннер не закрывает: там могут быть несохранённые правки.

**Файловая база.** Сеансы файловой базы не завершает ни один ключ: ни `-SessionTerminate
force`, ни `--session-terminate=force` у агента и `ibcmd` (замер #211 на 8.3.27 и 8.5.1).
Процессы 1С раннер не снимает. Поэтому у файловой базы `terminable: false`, `--sessions force`
отказывает так же и с тем же списком, блокировку `1Cv8.cdn` раннер не ставит, а агенту
`--session-terminate=force` не передаёт никогда: на нём агент не отказывает, а спрашивает и
ждёт ответа. `lock.state` — `foreign`, если `1Cv8.cdn` стоит, иначе `none`.

**Сервер — автономный и кластер.** `--sessions force` завершает сеансы ключом самого
исполнителя, без блокировки начала сеансов: `-SessionTerminate force` у Конфигуратора,
`--session-terminate=force` с обязательным `--session-terminate-message` у `ibcmd`,
`--session-terminate=force` у агента. Применение идёт сначала без ключа; после отказа — с ним.
`terminable: true`, `lock.state: unknown`: узнать о блокировке раннер не может. Исход
записывается, как только ключ отдан платформе, — и при удаче, и при отказе второго запуска. Если
кто-то успел войти, будет отказ, и вызов повторяют. Поведение агента на этом ключе у сервера не
замерено; если он спросит, раннер ответит «Отмена», и отказ назовёт сеансы.

**Ключ по умолчанию.** Без `force` `ibcmd config apply` получает явный
`--session-terminate=disable`; Конфигуратор и агент — ничего.

**Ответ после `force`.** Формы `push` и `apply` несут `sessions: {terminated, lock,
message_delivered}`: завершённые сеансы из списка первого отказа (пусто — завершать было
некого; `null` — список не получен), блокировку и то, передал ли раннер платформе сообщение
(только `ibcmd`).
