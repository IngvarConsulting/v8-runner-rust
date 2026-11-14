---
id: INV.USE-CASES.VANESSA-DOWNLOAD-IS-INDEPENDENT-OF-INFOBASE-READINESS
check:
  - tests/cli_tools_download.rs::tools_download_vanessa_without_an_origin
  - tests/cli_tools_download.rs::tools_download_vanessa_with_a_stopped_managed_cluster
  - tests/cli_tools_download.rs::tools_download_vanessa_validates_declared_and_explicit_infobases
---

# Загрузка Vanessa обходится без работающей ИБ

`tools download vanessa` скачивает EPF и обновляет `tools.va.epf_path` в локальном YAML
с сохранением остальных настроек. `origin` может быть ещё не объявлен. При объявленной
базе команда проверяет форму её настроек, включая явный выбор через `--infobase`.
Загрузка обходится без адреса работающего кластера, запуска служб и выбора провайдера 1С.
