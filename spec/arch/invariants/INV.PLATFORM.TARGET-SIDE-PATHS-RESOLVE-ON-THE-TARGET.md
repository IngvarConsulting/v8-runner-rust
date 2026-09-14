---
id: INV.PLATFORM.TARGET-SIDE-PATHS-RESOLVE-ON-THE-TARGET
status: planned
governs: product
decision: DEC.2026-09-14.THE-TARGET-MAY-LIVE-ON-ANOTHER-MACHINE
check: null
scope: [platform]
---

# Пути в командах цели разрешаются на её стороне

Каталог выгрузки и имя файла, переданные агенту или шлюзу, разрешаются на стороне цели. Раннер не складывает такой путь из своего рабочего каталога и не читает его напрямую.
