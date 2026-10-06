---
id: INV.USE-CASES.AN-UNRECOVERABLE-STATE-STORE-ERROR-IS-A-REFUSAL
check:
  - src/change_detection/analyzer.rs::hard_storage_errors_stay_hard_during_full_rescan
---

# Неустранимая ошибка хранилища состояния — отказ

Неустранимая ошибка хранилища состояния анализа изменений заканчивает команду отказом, а не
молчанием и не загрузкой наугад; полный пересмотр её не прячет.
