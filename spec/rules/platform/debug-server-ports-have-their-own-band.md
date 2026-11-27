---
id: INV.PLATFORM.DEBUG-SERVER-PORTS-HAVE-THEIR-OWN-BAND
check:
  - src/use_cases/managed_stand/debug_server.rs::port_windows_stay_in_their_own_band
  - src/use_cases/managed_stand/debug_server.rs::debug_start_does_not_publish_exited_unresponsive_or_foreign_processes
  - src/platform/ragent.rs::the_debug_port_follows_the_worker_range_inside_the_block
  - src/use_cases/managed_stand/cluster.rs::a_taken_debug_port_moves_no_registry
  - src/use_cases/managed_stand/standalone.rs::a_taken_debug_port_moves_the_block_only_with_the_debug_server
---

# У портов сервера отладки своя полоса

Свой `dbgs` слушает первый свободный порт окна из 32 портов в полосе 10000–19999: окно выводится из
пути `workPath`, а если оно занято целиком, берётся следующее. Полоса не задевает закреплённые порты стенда (20000 и
выше), порты сервера 1С по умолчанию (1540–1591) и динамический диапазон. Порт выбирает сам `dbgs` и называет его.
Если на выбранном порту ответит чужой процесс, раннер отказывает в выдаче аренды.

Сервер отладки кластера слушает порт следом за диапазоном рабочих процессов, внутри шага его закреплённого блока:
диапазон `/range` не меняется, потому что реестр кластера помнит прежний.

`dbgs` автономного сервера слушает порт следом за диапазоном прямых соединений, внутри шага его закреплённого
блока; при включённой отладке занятость этого порта переносит блок целиком (`INV.USE-CASES.THE-STANDALONE-DEBUG-SERVER-STAYS-ON-UNTIL-NO-DEBUG`).
