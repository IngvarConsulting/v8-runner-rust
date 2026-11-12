---
id: INV.USE-CASES.A-PUSH-NEVER-LOADS-AN-EXTERNAL-SET
check:
  - src/use_cases/build_project.rs::a_whole_ibcmd_push_never_imports_an_external_set
  - src/use_cases/build_project.rs::a_whole_ibcmd_push_preview_plans_no_load_of_an_external_set
  - src/use_cases/build_project.rs::a_whole_designer_push_never_loads_an_external_set
---

# Отправка целиком не грузит внешний набор в базу

Набор `EXTERNAL_DATA_PROCESSORS` или `EXTERNAL_REPORTS` проекта DESIGNER `push --full` в базу
не грузит ни Конфигуратором, ни `ibcmd`. У `ibcmd` это не зависит от того, как он выбран:
ключом `providers.push` или умолчанием по объёму. Артефакты набора находятся для упаковки, а
его шаг — успешный `mode: skipped`, `applied: false`. Превью `push --full` и `push --force`
через `ibcmd` планирует тот же пропуск, а ошибку в исходниках набора называет отказом проверки
на его шаге, а не планом импорта.
