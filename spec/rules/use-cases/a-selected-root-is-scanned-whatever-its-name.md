---
id: INV.USE-CASES.A-SELECTED-ROOT-IS-SCANNED-WHATEVER-ITS-NAME
check:
  - src/change_detection/scanner.rs::selected_roots_are_scanned_while_ignored_descendants_stay_excluded
  - src/change_detection/source_sets.rs::a_source_set_rooted_at_a_service_named_directory_is_analyzed
  - src/change_detection/source_sets.rs::a_generated_designer_copy_named_build_is_analyzed
---

# Выбранный корень обходится, как бы он ни назывался

Корень набора обходится и тогда, когда его имя совпадает с исключённым каталогом
(например, `build` или `tmp`): исключение относится к потомкам, а не к явно выбранному
набору (#302). Это относится и к порождённой копии набора EDT в `workPath/designer`.
