---
id: INV.CLI.A-PACKAGE-DIRECTORY-NEVER-OVERLAPS-THE-SOURCES
check:
  - tests/cli_make_download_all.rs::a_package_directory_overlapping_the_sources_is_refused_before_work
---

# Пакет обхода без набора не ложится на исходники и `workPath`

`make` и `download` без набора до первой сборки или выгрузки сверяют цель каждого пакета с
каталогом каждого набора проекта и с `workPath`. Цель, которая совпадает с одним из них,
лежит внутри него или вмещает его, — отказ родом `validation` до запуска платформы: публикация
каталога внешнего набора поверх его исходников заменила бы их. Исходники остаются на месте.
