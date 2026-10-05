---
id: INV.USE-CASES.A-VANESSA-JUNIT-REPORT-LANDS-IN-ITS-RUN
check:
  - tests/cli_test.rs::test_va_directs_nested_junit_directory_into_the_run
  - tests/cli_test.rs::test_va_creates_the_nested_junit_directory_when_the_template_lacks_it
  - src/use_cases/vanessa.rs::junit_dir_overlay_replaces_a_non_object_nested_report_section
---

# JUnit-отчёт Vanessa ложится в каталог своего прогона

`test va` направляет каталог JUnit-отчёта в каталог `junit` своего прогона в каждом поле
порождённых VAParams, откуда Vanessa Automation его читает: в верхнем
`КаталогВыгрузкиJUnit` и во вложенном `ОтчетJUnit.КаталогВыгрузкиJUnit`. Значения шаблона
в этих полях заменяются; вложенный объект создаётся, если шаблон его не задаёт или задаёт
не объектом, а прочие его ключи остаются. Так отчёт остаётся в артефактах прогона и
разбирается, какое бы поле ни читала установленная версия Vanessa.
