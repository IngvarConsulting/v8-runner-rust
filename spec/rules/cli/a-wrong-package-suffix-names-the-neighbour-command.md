---
id: INV.CLI.A-WRONG-PACKAGE-SUFFIX-NAMES-THE-NEIGHBOUR-COMMAND
check:
  - tests/cli_infobase.rs::infobase_dump_into_a_package_names_download
  - tests/cli_load.rs::upload_of_a_transfer_file_names_infobase_restore
---

# Чужое расширение файла называет соседнюю команду

`infobase dump --output main.cf` и `upload ib.dt` отказывают до запуска платформы, и
отказ называет команду для этого расширения: `download` и `infobase restore`.
