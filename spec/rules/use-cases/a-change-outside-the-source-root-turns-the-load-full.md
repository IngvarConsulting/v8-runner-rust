---
id: INV.USE-CASES.A-CHANGE-OUTSIDE-THE-SOURCE-ROOT-TURNS-THE-LOAD-FULL
check:
  - src/change_detection/partial_load.rs::traversal_or_symlink_escape_forces_full
  - src/change_detection/partial_load.rs::a_traversal_component_is_refused_even_when_it_resolves_inside
  - src/change_detection/partial_load.rs::a_removed_object_with_an_escaping_or_dangling_descendant_is_full
  - src/change_detection/partial_load.rs::unportable_unix_paths_cannot_escape_agent_staging
  - src/change_detection/partial_load.rs::a_path_beginning_with_an_option_prefix_is_not_a_partial_argument
---

# Изменение вне корня набора делает загрузку полной

Если путь изменённого файла выходит за корень исходников набора — через `..` или ссылку
наружу, — список частичной загрузки не составляется, и операция идёт полной загрузкой.

Проверка распространяется на раскрытые каталоги и существующих предков удалённых путей.
Полная загрузка также нужна, если путь нельзя однозначно передать в списке или аргументах
исполнителя.
