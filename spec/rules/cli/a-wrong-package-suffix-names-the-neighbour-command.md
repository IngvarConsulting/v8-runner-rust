---
id: INV.CLI.A-WRONG-PACKAGE-SUFFIX-NAMES-THE-NEIGHBOUR-COMMAND
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/192
---

# Чужое расширение файла называет соседнюю команду

`infobase dump --output main.cf` и `upload ib.dt` отказывают до запуска платформы, и
отказ называет команду для этого расширения: `download` и `infobase restore`.
