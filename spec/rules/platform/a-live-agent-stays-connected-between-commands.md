---
id: INV.PLATFORM.A-LIVE-AGENT-STAYS-CONNECTED-BETWEEN-COMMANDS
check:
  - tests/agent_keeper.rs::a_second_client_reuses_the_live_agent_without_a_new_launch
  - tests/cli_build_agent.rs::a_second_push_does_not_connect_the_live_agent_again
  - tests/cli_live_agent_portable.rs::a_second_command_takes_the_kept_connection_without_connect_ib
  - tests/cli_build_agent.rs::a_push_with_nothing_to_load_leaves_the_early_agent_connected_without_work
  - tests/agent_keeper.rs::a_disconnect_spares_a_session_and_releases_an_idle_agent
  - tests/cli_live_agent_portable.rs::another_executor_finds_the_infobase_released_by_the_live_agent
  - tests/cli_live_agent_portable.rs::agent_release_disconnects_the_live_agent_from_the_infobase
  - src/use_cases/transport.rs::the_boundary_decides_what_happens_to_the_connected_live_agents
  - tests/agent_keeper.rs::a_conditional_stop_spares_an_agent_in_use
  - tests/agent_keeper.rs::an_idle_agent_closes_after_its_idle_limit
  - tests/agent_keeper.rs::a_session_left_without_end_disconnects_the_agent
  - tests/agent_keeper.rs::the_first_read_on_a_stale_connection_reconnects_and_repeats
  - tests/agent_keeper.rs::a_critical_command_on_a_stale_connection_is_not_repeated_and_the_next_session_reconnects
  - tests/agent_keeper.rs::a_streamed_read_on_a_stale_connection_marks_it_for_the_next_session
  - src/use_cases/apply/sessions.rs::a_sessions_refusal_names_the_connected_live_agents_and_agent_release
---

# Живой агент остаётся подключённым к базе между командами

Подключение живого агента к базе (`common connect-ib`) живёт дольше команды. Следующая
команда того же ключа берёт его без `common connect-ib`, и её первая загрузка идёт в уже
подключённой сессии. На КА 8.3.27 `connect-ib` стоил 2,6 с, а первая загрузка модулей в
новой сессии шла 7,5 с против 1,6 с в подключённой (разбор в #599, замер на сборке #634).

Подключённый агент служит замком файловой базы: пока он подключён, пакетный Конфигуратор,
`ibcmd` и другой агент конфигурацию в неё не загружают — платформа отказывает блокировкой базы
для конфигурирования, — а сеанс Предприятия пускается
([замер](../../../references/1c/confirmed-runtime-measurements.md), раздел о подключённом агенте
как замке файловой базы). Поэтому чужая загрузка не меняет базу между командами, пока агент
подключён. У серверной базы это не замерено.

Подключение живёт с агентом, а не с работой. `push`, которому разбор не нашёл работы, агента
не отключает: тот остаётся подключённым, как после любой команды. Держимый агент
(`agent hold`) остаётся подключённым, пока его держат, без срока простоя.

Агент отключается от базы:

- когда его снимают: он без держателя и простоял 15 минут
  (`INV.PLATFORM.A-LIVE-AGENT-CLOSES-WHEN-NOBODY-HOLDS-IT`), его сняли `agent stop` или
  предел, прервали команду;
- когда команда ушла, не закрыв сессии: в каком состоянии она оставила агента, неизвестно;
- по `v8-runner agent release`: он снимает отметку держателя и отключает агента, если тот не
  занят командой. Это выход для чужого Конфигуратора и чужой монопольной операции: агент
  держит сеанс Конфигуратора базы, у файловой базы — `1Cv8.cgr`, и сеанс лицензии;
- когда базой займётся не он. Команда, выбравшая для операции другого исполнителя (пакетный
  Конфигуратор, `ibcmd`, разовый или подключённый агент), и команда, открывающая базу
  клиентом 1С (`test`, `launch`), сначала отключают от неё простаивающих живых агентов этой
  базы. Свой живой агент отключает агентов этой базы с другим ключом. `infobase restore` и
  пересоздание базы снимают простаивающих агентов, подключённых или нет. Остановка и
  перезапуск кластера раннера и его СУБД отключают агентов серверных баз. Так отключаются и
  обновление с реструктуризацией другим исполнителем, и `--sessions force`, и `reset`.

Агента, занятого командой или сессией другой команды, не отключают: команда, которой нужна
база, получает отказ `infobase_busy` (`INV.PLATFORM.ANOTHER-LAUNCH-MODE-WAITS-ONLY-FOR-A-BUSY-AGENT`).
Пока агент подключается к базе без сессии (`push` во время разбора), он тоже занят, и чужая
команда этой базы в эти секунды получает тот же отказ.

Подключение могло устареть между командами: сеанс завершили, сервер перезапустили. Первая
некритическая команда сессии на взятом подключении, получив отказ платформы «Конфигуратор не
подключен к информационной базе» (`DesignerNotConnectedToInfoBase`) одним массивом ответа,
переподключает агента (`disconnect-ib`, `connect-ib`) и повторяется один раз; повтор пишется в
журнал сторожа. Иной отказ не повторяется. Команда, которая так отказала, не будучи повторённой —
критическая или уже приславшая часть ответа, — доводит отказ до команды, а следующая сессия
подключается заново. Каким ответом платформа встречает устаревшее подключение, решает замер
(`INV.PLATFORM.A-STALE-KEPT-CONNECTION-IS-RECOGNISED`).

Отказы «базу держат» (`infobase_sessions`, `infobase_designer_busy`) называют живых агентов
этой базы, которые остаются подключёнными, и выход `v8-runner agent release`, не утверждая,
что держат именно они.

Остатки:

- раннер сборки до этого правила со сторожем новой сборки не встречается: сборка раннера входит
  в ключ живого агента (`INV.PLATFORM.A-LIVE-AGENT-IS-TAKEN-ONLY-BY-ITS-KEY`). Но он не знает
  поля `attached` и подключённого агента новой сборки той же базы считает простаивающим без
  подключения: не отключает его, и его пакетный Конфигуратор или `ibcmd` упирается в
  блокировку базы — у файловой базы отказ `infobase_designer_busy` «закройте Конфигуратор»,
  агента он не называет. Выход — `v8-runner agent release` новой сборкой;
- голый отказ исполнителя без признака сеансов агента не называет;
- держит ли подключённый агент блокировку конфигурирования серверной базы после
  `push --no-apply`, не замерено.

Тесты из `tests/cli_build_agent.rs` идут только на Unix, а тесты из
`tests/cli_live_agent_portable.rs` в Job без права выхода пропускаются: их проверяет CI.
