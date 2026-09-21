---
id: INV.CLI.A-WRONG-PACKAGE-SUFFIX-NAMES-THE-NEIGHBOUR-COMMAND
status: planned
governs: product
decision: DEC.2026-09-21.A-PACKAGE-IS-UPLOADED-AND-DOWNLOADED
check: null
scope: [cli]
---

# Чужое расширение файла называет соседнюю команду

`infobase dump --output main.cf` и `upload ib.dt` отказывают до запуска платформы, и
отказ называет команду для этого расширения: `download` и `infobase restore`.
