---
id: INV.USE-CASES.VANESSA-PRERELEASE-IS-TAKEN-ONLY-ON-REQUEST
check:
  - tests/cli_tools_download.rs::tools_download_vanessa_takes_the_latest_release_without_prerelease_flag
  - tests/cli_tools_download.rs::tools_download_vanessa_prerelease_takes_the_highest_version_including_prerelease
  - src/use_cases/tools_download.rs::release_versions_compare_numerically_by_component
---

# Vanessa берёт pre-release только по просьбе

Без ключа `tools download vanessa` берёт выпуск, который GitHub отдаёт как
`releases/latest`; pre-release туда не попадает, даже если его версия больше (#160).

С ключом `--prerelease` команда берёт выпуск с наибольшей версией среди всех
опубликованных выпусков `Pr-Mex/vanessa-automation-single`, pre-release тоже. Список
выпусков читается со всех страниц, черновики пропускаются. Версия читается из тега:
компоненты через точку, каждый сравнивается как число, ведущая `v` допустима; поэтому
1.2.043.42 больше 1.2.043.9. Тег, который так не читается, в выборе не участвует.
