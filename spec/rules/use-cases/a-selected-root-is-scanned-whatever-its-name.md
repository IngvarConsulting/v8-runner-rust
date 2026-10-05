---
id: INV.USE-CASES.A-SELECTED-ROOT-IS-SCANNED-WHATEVER-ITS-NAME
check: [src/change_detection/scanner.rs::selected_roots_are_scanned_while_ignored_descendants_stay_excluded]
---

# Выбранный корень обходится, как бы он ни назывался

Корень набора обходится и тогда, когда его имя совпадает с исключённым каталогом
(например, `build` или `tmp`): исключение относится к потомкам, а не к явно выбранному
набору (#302).
