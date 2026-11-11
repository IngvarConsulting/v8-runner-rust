---
id: INV.PLATFORM.A-LIVE-AGENT-STARTS-WHILE-THE-COMMAND-PREPARES
check:
  - tests/agent_keeper.rs::an_agent_started_idle_connects_only_when_a_session_begins
  - tests/cli_build_agent.rs::a_push_with_nothing_to_load_leaves_the_early_agent_connected_without_work
  - tests/agent_keeper.rs::an_attach_connects_without_a_session_and_the_session_takes_it
  - tests/agent_keeper.rs::an_attach_with_other_credentials_removes_the_agent
  - tests/cli_live_agent.rs::a_pull_the_guard_refuses_answers_alike_and_sends_the_platform_no_work
  - src/use_cases/agent_session.rs::an_early_agent_the_command_did_not_take_is_let_go_at_once
  - src/use_cases/agent_session.rs::an_early_agent_of_another_key_is_not_taken
  - src/use_cases/agent_session.rs::an_early_agent_removed_before_its_session_is_taken_again
  - src/use_cases/agent_session.rs::a_refusal_at_the_session_begin_answers_like_one_at_the_launch
  - src/platform/agent_keeper/client.rs::a_start_the_command_stopped_waiting_for_keeps_the_keeper
  - src/platform/agent_keeper/client.rs::a_begin_the_command_stopped_waiting_for_returns_at_once
  - src/platform/agent_keeper/client.rs::a_start_line_without_at_open_connects_the_agent
  - src/platform/agent_keeper/client.rs::a_cancelled_start_returns_at_once_and_terminates_the_keeper
  - src/use_cases/dump_config.rs::a_preview_an_extension_and_live_off_raise_no_early_agent
  - tests/agent_keeper.rs::an_agent_started_idle_that_cannot_connect_is_removed
---

# Живой агент поднимается, пока команда готовится

`pull` и `push` с исходниками Конфигуратора, у которых живой агент управляемой точки входа
по дешёвому прогнозу станет исполнителем, поднимают его в начале команды, а свою подготовку —
отказы, сторож замены каталога, сверку файла версий, перечень дерева, разбор изменений — ведут
тем временем. Первая рабочая команда платформе ждёт конца подготовки. Агента поднимают без
подключения к базе: `pull` сразу открывает сессию, и агент подключается ею; `push` подключает
его к базе без сессии команды, пока идёт разбор, а сессию открывает, только когда разбор нашёл
работу (`INV.PLATFORM.AGENT-SESSION-LIVES-WITH-THE-LOCK`); без работы агент остаётся
подключённым. Агент, уже подключённый к базе,
подключения не повторяет (`INV.PLATFORM.A-LIVE-AGENT-STAYS-CONNECTED-BETWEEN-COMMANDS`). Превью и
выгрузка расширения агента заранее не поднимают; отправка, формат которой новее платформы,
тоже: она отказывает до запуска платформы
(`INV.USE-CASES.A-LOAD-FROM-A-NEWER-FORMAT-IS-REFUSED-BEFORE-THE-PLATFORM-STARTS`).

Отказ подготовки, выбор другого исполнителя и отмена отвечают тем же, что и без раннего
подъёма: команда перестаёт ждать агента, закрывает открытую сессию, а поднятый агент живёт по
обычным правилам живого агента. Отказ подготовки сторожа не снимает, даже если тот ещё не
ответил; отмена команды снимает сторожа, который она подняла и который ещё не ответил, как и
без раннего подъёма. Другому исполнителю агент уступает до его работы: начатое подключение к
базе доводится и снимается. Ранее работавший агент только отключается от базы. Отказ
подключения у агента, поднятого без подключения, — при начале сессии или при подключении без
сессии — отвечает тем же кодом, что отказ подъёма, и такой агент, ни разу не подключившийся к
базе, снимается, как не поднявшийся: сеанса лицензии он не держит. Агента, которого сторож за это время снял, команда берёт или поднимает заново.

Только живой агент: с `live: false` и в Job команда работает с разовым агентом, как прежде, —
поднимает его, когда подготовка кончилась, и снимает после работы.
