---
id: INV.CLI.MAKE-AND-DOWNLOAD-WITHOUT-A-SET-STOP-AT-THE-FIRST-FAILURE
check:
  - tests/cli_make_download_all.rs::make_without_a_set_stops_at_the_first_failed_set
  - tests/cli_make_download_all.rs::download_without_a_set_stops_at_the_first_failed_set
---

# Отказ набора останавливает обход `make` и `download`

Отказ при сборке или выгрузке набора останавливает `make` и `download` без набора: наборы
после него не обрабатываются, ответ несёт сделанное до отказа и отказавший набор последней
записью `sets`.
