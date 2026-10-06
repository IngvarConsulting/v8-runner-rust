---
id: INV.CLI.PULL-ALL-GIVES-NO-SECOND-SET-TO-AN-EXTENSION-A-SET-HOLDS
check:
  - tests/cli_pull_all.rs::a_set_holding_an_extension_under_another_name_gets_no_second_set
  - src/use_cases/dump_config/all.rs::a_set_holding_an_installed_extension_under_another_name_is_not_doubled
---

# Расширение, которое держит набор под другим именем, второго набора не получает

Набор расширения, исходники которого (`Name` в `Configuration.xml`, у EDT — в
`Configuration.mdo`) называют установленное расширение не тем именем, каким набор называет
его платформе, — `pull --all` отказывает до первой выгрузки и просит переименовать набор, а
второго набора для того же расширения не объявляет. Пока имя набора не стало псевдонимом
([#218](https://github.com/IngvarConsulting/v8-runner-rust/issues/218)), раннер называет
расширение платформе именем набора.
