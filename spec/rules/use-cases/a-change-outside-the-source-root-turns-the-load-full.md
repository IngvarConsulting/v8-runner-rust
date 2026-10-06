---
id: INV.USE-CASES.A-CHANGE-OUTSIDE-THE-SOURCE-ROOT-TURNS-THE-LOAD-FULL
check:
  - src/change_detection/partial_load.rs::traversal_or_symlink_escape_forces_full
---

# Изменение вне корня набора делает загрузку полной

Если путь изменённого файла выходит за корень исходников набора — через `..` или ссылку
наружу, — список частичной загрузки не составляется, и операция идёт полной загрузкой.
