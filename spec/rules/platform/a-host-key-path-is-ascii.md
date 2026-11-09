---
id: INV.PLATFORM.A-HOST-KEY-PATH-IS-ASCII
check:
  - src/use_cases/agent_session.rs::a_one_time_host_key_gets_an_ascii_path_from_a_cyrillic_work_path
  - src/use_cases/agent_session.rs::a_declared_host_key_outside_ascii_is_refused_before_the_launch
---

# Агент получает ключ хоста по пути в ASCII

Агент Конфигуратора читает файл `/AgentSSHHostKey` только по пути в ASCII. Если в пути есть
кириллица, он пишет в `/Out` «Фатальная ошибка SSH-сервера: Ошибка установки ключа хоста» и
выходит, не открыв сессию. Кириллица в `AgentBaseDir` и пробел в пути не мешают
([замер](../../../references/1c/confirmed-runtime-measurements.md), раздел о ключе хоста по пути
не в ASCII).

Поэтому раннер выбирает каталог одноразового ключа так:
- если путь к `workPath` в ASCII — `workPath/agent/host-keys`;
- иначе — каталог учётной записи во временном каталоге ОС;
- иначе, на Windows, — в `%ProgramData%\v8-runner`.

Чужой каталог создаётся доступным только владельцу и назван по хешу профиля учётной записи. Если
ни один путь не в ASCII, раннер отказывает до запуска и называет причину.

Объявленный `tools.designer_agent.host-key` по пути не в ASCII — отказ `environment_unavailable`
до запуска агента, называющий путь, а не «агент вышел».
