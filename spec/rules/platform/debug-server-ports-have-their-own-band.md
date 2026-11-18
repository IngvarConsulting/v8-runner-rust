---
id: INV.PLATFORM.DEBUG-SERVER-PORTS-HAVE-THEIR-OWN-BAND
check:
  - src/use_cases/managed_stand/debug_server.rs::port_windows_stay_in_their_own_band
  - src/use_cases/managed_stand/debug_server.rs::a_taken_explicit_port_is_refused_before_anything_starts
  - src/use_cases/managed_stand/debug_server.rs::a_port_taken_during_the_start_is_named_as_taken
  - src/platform/ragent.rs::the_debug_port_follows_the_worker_range_inside_the_block
---

# У портов сервера отладки своя полоса

Без `--port` свой `dbgs` слушает первый свободный порт окна из 32 портов в полосе 10000–19999: окно выводится из
пути `workPath`, а если оно занято целиком, берётся следующее. Полоса не задевает закреплённые порты стенда (20000 и
выше), порты сервера 1С по умолчанию (1540–1591) и динамический диапазон. Порт выбирает сам `dbgs` и называет его.
Явный `--port`, занятый другим процессом, — отказ до запуска, а слушающий процесс остаётся работать; если порт заняли
между проверкой и запуском, отказ тот же. Текст журнала `dbgs` для этого не разбирается.

Сервер отладки кластера слушает порт следом за диапазоном рабочих процессов, внутри шага его закреплённого блока:
диапазон `/range` не меняется, потому что реестр кластера помнит прежний.
