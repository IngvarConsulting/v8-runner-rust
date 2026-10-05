---
id: CTR.USE-CASES.VANESSA-DOWNLOAD-TAKES-THE-HIGHEST-STABLE-RELEASE
check:
  - tests/cli_tools_download.rs::tools_download_vanessa_takes_the_highest_stable_release_not_the_latest_flag
  - src/use_cases/tools_download.rs::release_versions_compare_numerically_by_component
---

# `tools download vanessa` берёт наибольший обычный выпуск

«Последняя» Vanessa Automation single — выпуск с наибольшей версией среди выпусков
`Pr-Mex/vanessa-automation-single` без пометки pre-release; черновики тоже не в счёт.
Флаг latest на GitHub не учитывается: его ставит автор выпуска, и он оставался на
1.2.043.1, когда уже вышла 1.2.043.42 (#160). Выбор идёт по всему списку выпусков, со
всех его страниц.

Версия читается из тега: компоненты через точку, каждый сравнивается как число, ведущая
`v` допустима. Поэтому 1.2.043.42 больше 1.2.043.9. Тег, который так не читается, в
выборе не участвует; если не читается ни один — команда отказывает, а не берёт что
попало.

Ответ называет выбранную версию полем `tag` в `destinations` формы
`CTR.WIRE.TOOLS-DOWNLOAD-DATA`.

Правило касается только Vanessa: YAxUnit и onec-client-mcp-devkit по-прежнему берут
выпуск, помеченный latest.
