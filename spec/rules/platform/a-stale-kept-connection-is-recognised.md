---
id: INV.PLATFORM.A-STALE-KEPT-CONNECTION-IS-RECOGNISED
check:
  - src/platform/rac.rs::only_a_complete_snapshot_of_the_expected_base_can_exclude_designer
  - tests/agent_keeper.rs::an_unknown_error_without_a_managed_snapshot_reaches_the_client
  - tests/agent_keeper.rs::the_first_read_on_a_stale_connection_reconnects_and_repeats
---

# Сторож распознаёт потерю соединения по машинному ответу

На 1С 8.3.27.2342 завершение сеанса через `rac session terminate` оставляет SSH
открытым. Агент возвращает явный `UnknownError`, текст которого зависит от параметров
завершения сеанса. После прямых перезапусков кластера и PostgreSQL измеренные
команды продолжали успешно работать в прежнем SSH
([замеры #754](https://github.com/IngvarConsulting/v8-runner-rust/pull/754)).

Для первой команды на сохранённом подключении `DesignerNotConnectedToInfoBase`
по-прежнему подтверждает потерю соединения. Явный `UnknownError` подтверждает её
только в кластере раннера, когда успешный полный снимок `rac session list` для
точной ИБ не содержит ни одного сеанса `Designer`, включая спящие. При отказе RAC,
неполном ответе, наличии сеанса Designer или неподтверждённой цели сторож возвращает
исходный отказ. Текст отказа агента не участвует в проверке.

Непрерванная первая некритическая команда без уже переданного ответа повторяется
один раз после переподключения. Критическая команда или команда, уже передавшая
часть ответа, возвращает отказ. Следующая сессия переподключается.
