---
id: INV.USE-CASES.DOUBT-TURNS-A-PARTIAL-LOAD-INTO-A-FULL-ONE
check:
  - src/change_detection/partial_load.rs::decide_forces_full_when_deleted_files_exist
  - src/change_detection/partial_load.rs::decide_forces_full_when_configuration_xml_changed
  - src/change_detection/partial_load.rs::decide_forces_full_when_changed_path_is_directory
  - src/change_detection/partial_load.rs::traversal_or_symlink_escape_forces_full
  - src/change_detection/analyzer.rs::hard_storage_errors_stay_hard_during_full_rescan
---

# Сомнение переводит частичную загрузку в полную

Частичная загрузка допустима только при уверенности в наборе изменённых файлов. Удаление,
правка корневого описания `Configuration.xml`, изменённый каталог и выход набора за корень
исходников — через `..` или ссылку наружу — переводят операцию в полную. Неустранимая
ошибка хранилища состояния — отказ, а не молчание.
