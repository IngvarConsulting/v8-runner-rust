---
id: INV.PLATFORM.REMOTE-FILES-TRAVEL-BY-A-DECLARED-CHANNEL
check:
  - tests/cli_agent_standalone.rs::a_standalone_server_without_a_declared_channel_is_refused_before_any_session
  - tests/cli_agent_standalone.rs::an_agent_chosen_next_to_the_direct_gate_without_a_channel_is_refused_before_any_session
  - src/config/validate.rs::a_standalone_target_takes_either_way_and_refuses_neither
  - tests/cli_standalone_direct_gate.rs::a_standalone_server_with_only_the_direct_gate_is_served_by_the_designer
---

# Файлы удалённой цели забираются объявленным каналом

Результат, оставшийся на стороне автономного сервера после команды агента через SSH-шлюз,
переносится объявленным каналом обмена `standalone.exchange`. Чтение чужого пути так, будто
файловая система общая, запрещено. Без канала сессия SSH-шлюза не открывается: секция с
`standalone.gate` без строки прямого шлюза и без канала — ошибка валидации, а агент,
выбранный рядом со строкой, получает тот же отказ до обращения к шлюзу.

Конфигуратору по прямому шлюзу канал не нужен: файлы команды остаются у раннера, и секция
`standalone` со строкой прямого шлюза без канала принимается.
