---
id: INV.CHANGE-DETECTION.RUNNER-DIRECTORIES-ARE-NOT-SOURCES
check:
  - src/change_detection/runner_dirs.rs::both_runner_directories_inside_the_root_are_named_whatever_their_names
  - src/change_detection/runner_dirs.rs::runner_directories_are_named_only_here
  - src/change_detection/runner_dirs.rs::a_path_overlaps_a_runner_directory_in_either_direction
  - src/change_detection/scanner.rs::a_work_path_inside_the_root_is_not_a_source_whatever_its_name
  - src/change_detection/scanner.rs::the_default_local_directory_is_skipped_by_its_name
  - src/change_detection/analyzer.rs::a_personal_file_in_a_local_path_inside_the_set_is_not_a_change
  - src/change_detection/partial_load.rs::a_build_list_leaves_the_runner_directories_out
  - src/use_cases/agent_session.rs::a_root_copied_for_the_agent_leaves_the_agent_directory_behind
  - src/use_cases/dump_config.rs::only_full_pull_refuses_a_target_containing_local_path
---

# Каталоги раннера — не исходники

`workPath` и `localPath` внутри корня набора, с любым именем, исходниками не являются. Какие из
них лежат под корнем, отвечает один владелец, `runner_dirs_inside`. Его спрашивают сканер,
анализатор, частичная загрузка сборки, копия набора для агента и полная выгрузка: она отказывает
над каталогом, который содержит каталог раннера, потому что заменила бы его вместе с ним.
Каталог стадии внешней публикации и выход `convert --output` не пересекаются с каталогами
раннера.

`.local` пропускается и по имени, как остальные порождённые каталоги: это вторая защита
`localPath` по умолчанию у контекста, собранного без настроек. Прямой вызов прежнего
`work_path_inside` и прямое сравнение пути с `workPath` мимо владельца ловит сторож.
