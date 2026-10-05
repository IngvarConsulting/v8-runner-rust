---
id: INV.CLI.PULL-FORCE-IS-A-FULL-REPLACEMENT
check:
  - tests/cli_dump.rs::an_explicit_request_replaces_the_directory_and_keeps_nothing
---

# `pull --force` — полная выгрузка с заменой каталога

`pull --force` выгружает конфигурацию целиком и заменяет каталог набора состоянием базы:
незафиксированное там пропадает без копии.
