---
id: INV.PLATFORM.A-HOST-KEY-PATH-IS-ASCII
check:
  - src/platform/agent.rs::a_one_time_host_key_leaves_a_cyrillic_work_path_for_a_private_ascii_directory
  - src/use_cases/agent_session.rs::a_declared_host_key_outside_ascii_is_refused_before_the_launch
---

# Агент получает ключ хоста по пути в ASCII

Агент Конфигуратора читает файл `/AgentSSHHostKey` только по пути в ASCII. Если в пути есть
кириллица, он пишет в `/Out` «Фатальная ошибка SSH-сервера: Ошибка установки ключа хоста» и
выходит, не открыв сессию. Кириллица в `AgentBaseDir` и пробел в пути не мешают
([замер](../../../references/1c/confirmed-runtime-measurements.md), раздел о ключе хоста по пути
не в ASCII).

Поэтому на Windows одноразовый ключ лежит в `workPath/agent/host-keys`, только если путь к нему в
ASCII. Иначе раннер кладёт ключ в новый каталог со случайным именем под первым корнем с путём в
ASCII: временный каталог ОС, затем `%ProgramData%\v8-runner`. Каталог создаётся только для
владельца и только этим запуском, поэтому подложенный заранее чужой каталог не годится. Уходит
каталог вместе с ключом. Если такого корня нет, раннер отказывает с кодом `environment_unavailable`
до запуска и называет все испробованные пути.

Объявленный `tools.designer_agent.host-key` по пути не в ASCII — отказ `environment_unavailable`
до запуска агента, называющий путь, а не «агент вышел». Другие ОС не замерены, и там путь ключа
не ограничивается.
