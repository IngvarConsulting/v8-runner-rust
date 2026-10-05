---
id: INV.USE-CASES.VANESSA-PRERELEASE-TAKES-THE-HIGHEST-VERSION
check:
  - tests/cli_tools_download.rs::tools_download_vanessa_prerelease_takes_the_highest_version_including_prerelease
  - src/use_cases/tools_download.rs::release_versions_compare_numerically_by_component
---

# С `--prerelease` Vanessa берёт наибольшую версию

С ключом `--prerelease` команда `tools download vanessa` берёт выпуск с наибольшей
версией среди всех опубликованных выпусков `Pr-Mex/vanessa-automation-single`,
pre-release тоже. Список выпусков читается со всех страниц, черновики пропускаются.
Версия читается из тега: компоненты через точку, каждый сравнивается как число, ведущая
`v` допустима; поэтому 1.2.043.42 больше 1.2.043.9. Тег, который так не читается, в
выборе не участвует.
