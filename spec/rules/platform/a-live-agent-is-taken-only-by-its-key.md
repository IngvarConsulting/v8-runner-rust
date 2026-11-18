---
id: INV.PLATFORM.A-LIVE-AGENT-IS-TAKEN-ONLY-BY-ITS-KEY
check:
  - tests/agent_keeper.rs::other_credentials_remove_the_agent
  - tests/agent_keeper.rs::a_hold_with_other_credentials_removes_the_agent
  - tests/agent_keeper.rs::an_attach_with_other_credentials_removes_the_agent
  - src/use_cases/agent_session.rs::a_live_agent_of_another_build_has_another_key
  - tests/cli_live_agent_portable.rs::agent_stop_removes_the_live_agent_and_status_lists_it
  - tests/cli_live_agent_portable.rs::the_live_agent_lives_in_the_working_copy_and_comes_back_when_its_home_is_gone
  - src/platform/agent_keeper/mod.rs::the_home_of_a_live_agent_is_a_function_of_the_working_copy_and_the_key
---

# Живого агента берут только по его ключу

Ключ живого агента считается из строки подключения базы, версии платформы, пользователя
базы, `workPath`, версии и сборки раннера и учётной записи ОС. Сборку называет отметка
исполняемого файла (размер и время изменения): сторож другой сборки той же версии говорит
другим протоколом, и команда его не берёт. Команда берёт живого агента, только если
совпали все части ключа. Ключ — это HMAC с солью рабочей копии, а соль лежит в
`localPath/agent`, в каталоге, доступном только учётной записи. В открытом виде ключ не
хранится, потому что в строке подключения бывает пароль. Пароль базы в ключ не входит. Учётные данные команда передаёт сторожу с каждым
запросом. Если они не совпали с теми, с которыми открыта сессия, сторож снимает агента, и
команда поднимает нового.

Поскольку `workPath` входит в ключ, у каждой рабочей копии свой агент. Его запись,
`AgentBaseDir`, текущий каталог и ключ хоста лежат в его доме в рабочей копии,
`workPath/agent/live/<первые 16 знаков ключа>`
(`INV.PLATFORM.RUNNER-FILES-STAY-IN-THE-WORKING-COPY`). Каталог состояния учётной записи
под `%LOCALAPPDATA%` не годится: агент с `AgentBaseDir` там не выгружает файлы с `--update` за
точкой соединения ([замер #735](../../../references/1c/confirmed-runtime-measurements.md)). Пока
живой агент жив, его сторож держит `workPath/agent/live` (на Windows рабочую копию тогда не
удалить): уборка начинается с `v8-runner agent stop`, а сторож, уходя, убирает дом агента.
Каталог, в который агент выгружает файлы конфигурации, раннер выставляет в его каталоге
ссылкой, как и у разового агента. Файл (`.cf`, `.dt`) агент по пути через ссылку не пишет,
поэтому для файла раннер создаёт настоящий каталог в каталоге агента и потом переносит файл в
`workPath`. Файлы агента не попадают в исходники проекта
(`INV.PLATFORM.A-MANAGED-AGENT-RUNS-IN-ITS-OWN-DIRECTORY`). Запись хранит путь базы в
каноническом виде. Живых агентов одной базы из разных рабочих копий находят по каналам их
сторожей: каждый сторож называет свой дом, и там читается его запись.
Команду и агента связывает блокировка рабочего каталога, под которой команда держит агента
(`INV.PLATFORM.AGENT-SESSION-LIVES-WITH-THE-LOCK`).
