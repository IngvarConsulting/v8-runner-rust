---
id: INV.CLI.PULL-ALL-NAMES-AN-EXTENSION-WHOSE-DIRECTORY-OVERLAPS-A-SET
check:
  - src/use_cases/dump_config/all.rs::a_directory_overlapping_a_project_set_is_named_not_declared
  - src/use_cases/dump_config/all.rs::an_equal_directory_is_left_to_the_plan_check_despite_an_earlier_overlap
  - tests/cli_pull_all.rs::an_extension_whose_directory_overlaps_a_set_is_named_and_the_rest_is_pulled
---

# Каталог нового набора не пересекается с каталогом набора проекта

Если каталог `src/ext/<Name>` расширения без набора лежит внутри каталога набора проекта
или вмещает его, `pull --all` набор не объявляет и не выгружает: полная выгрузка внешнего
каталога заменила бы вложенный. Расширение называется в `data.not_declared` с причиной,
которая называет пересекающийся набор и советует объявить набор вручную под другим путём;
остальные наборы выгружаются и объявляются. Каталог, совпадающий с каталогом набора, —
отказ проверки плана, а не пропуск.
