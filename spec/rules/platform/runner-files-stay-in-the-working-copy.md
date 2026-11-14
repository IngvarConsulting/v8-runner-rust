---
id: INV.PLATFORM.RUNNER-FILES-STAY-IN-THE-WORKING-COPY
check:
  - tests/architecture_guardrails.rs::the_runner_creates_nothing_outside_the_working_copy
  - tests/architecture_guardrails.rs::the_working_copy_guard_sees_every_spelling
  - tests/cli_live_agent_portable.rs::the_live_agent_lives_in_the_working_copy_and_comes_back_when_its_home_is_gone
  - tests/agent_keeper.rs::stop_releases_and_removes_the_agent_and_its_record
  - tests/agent_keeper.rs::a_keeper_whose_home_is_gone_closes_itself
  - tests/agent_keeper.rs::the_keeper_names_its_home_in_the_working_copy
  - src/platform/live_registry.rs::the_registry_lock_is_held_by_one_holder_at_a_time
  - src/platform/live_registry.rs::a_listed_channel_lives_as_long_as_its_keeper
  - src/platform/agent_keeper/mod.rs::the_home_of_a_live_agent_is_a_function_of_the_working_copy_and_the_key
  - src/platform/process.rs::a_process_without_its_own_directory_starts_in_the_policy_directory
  - src/use_cases/version_file.rs::a_full_dump_takes_the_description_out_of_the_stage_before_publication
---

# Всё, что создаёт раннер, лежит в рабочей копии

Файлы и каталоги, которые создаёт раннер, лежат в рабочей копии: в `workPath`, если их можно
пересоздать, или в `localPath`, если они должны пережить его удаление. В каталоги учётной
записи ОС (`%LOCALAPPDATA%`, `%APPDATA%`, `%ProgramData%`) и во временный каталог ОС раннер
не пишет, кроме исключений ниже.

- Живой агент — в `workPath/agent/live`. Его дом `<первые 16 знаков ключа>/` содержит запись,
  `AgentBaseDir`, текущий каталог агента, ключ хоста и журналы процесса агента. Рядом с домом
  лежат вывод и замок записи сторожа, а в `bin/` — копия раннера, из которой сторож запущен.
  Путь дома считает одна функция рабочей копии и ключа, и ею пользуются и сторож, и команда.
  Соль ключа живого агента и отпечатка учётных данных лежит в `localPath/agent`.
- Временные файлы лежат в `workPath/temp`. Процесс платформы, которого команда ждёт
  (Конфигуратор, `ibcmd`, EDT, `psql`, …), начинает в `workPath/temp`, если раннер не назначил
  ему другого текущего каталога, а не в каталоге, откуда запущен раннер. Временный индекс git
  лежит в каталоге git рабочей копии и уходит после вопроса.

Сторож живого агента держит своим текущим каталогом `workPath/agent/live`, а запущен он из
копии раннера в `bin/`. Поэтому на Windows, пока агент жив, рабочую копию не удалить, и на любой
ОС уборка рабочей копии начинается с `v8-runner agent stop`. Сторож, уходя, убирает дом агента;
вывод сторожа остаётся рядом. Если дом убрали, пока сторож жив, сторож уходит сам, доведя
начатое, а следующая команда ждёт его ухода и поднимает нового агента.

Общего файла учётной записи нет. Живых агентов учётной записи для `live-max`, `agent status
--all` и `agent stop --all` раннер находит по каналам их сторожей, а запись читает в доме,
который называет сам сторож. Подсчёт и подъём идут под замком учётной записи, который файлом
не является на Windows.

Исключения:

- замок и метка владельца внешней файловой базы лежат рядом с базой, иначе другие рабочие
  копии их не увидят;
- файлы обмена на внешнем шлюзе `ibsrv`, у подключённого агента и на веб-сервере лежат там,
  где их читает точка входа, и убираются после каждой команды;
- на Unix сокет сторожа и замок реестра лежат в каталоге состояния учётной записи
  (`$XDG_STATE_HOME/v8-runner/keepers`), потому что путь сокета ограничен примерно 104 байтами.
  Данных там нет. На Windows каналы сторожей и замок реестра — именованные каналы, а не файлы;
- если путь каталога ключей в `workPath` не в ASCII, ключ хоста агента ложится во временный
  каталог ОС (`INV.PLATFORM.A-HOST-KEY-PATH-IS-ASCII`) и удаляется, как только агент открыл с
  ним первую сессию;
- фоновое удаление прежнего дерева выгрузки работает с текущим каталогом исполняемого файла
  раннера, который этот процесс и так держит: `workPath` и проект оно не держит;
- клиент 1С, которого `launch` оставляет работать после команды, начинает в каталоге, откуда
  запущен раннер: он живёт дольше команды и `workPath` держать не должен.

Собственные кэши платформы 1С (`%APPDATA%\1C`, `%LOCALAPPDATA%\1C`) — не файлы раннера, и
правило их не касается. То, что 0.17.0–0.17.3 оставили в `%LOCALAPPDATA%\v8-runner`
(`$XDG_STATE_HOME/v8-runner`), раннер убирает при подъёме живого агента. Каталоги агентов,
сторожа которых ещё живы, он не трогает.
