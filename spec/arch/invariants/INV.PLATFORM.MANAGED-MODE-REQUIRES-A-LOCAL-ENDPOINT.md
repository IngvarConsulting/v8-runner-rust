---
id: INV.PLATFORM.MANAGED-MODE-REQUIRES-A-LOCAL-ENDPOINT
status: planned
governs: product
decision: DEC.2026-09-14.THE-TARGET-MAY-LIVE-ON-ANOTHER-MACHINE
check: null
scope: [platform, config]
---

# Режим с запуском процесса требует локальной точки входа

Раннер поднимает процесс только там, где он вправе это сделать, — на своей машине. Удалённая точка входа в этом режиме отклоняется с названной причиной, а не обрабатывается как локальная.
