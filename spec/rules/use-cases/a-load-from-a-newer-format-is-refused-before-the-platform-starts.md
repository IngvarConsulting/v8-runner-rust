---
id: INV.USE-CASES.A-LOAD-FROM-A-NEWER-FORMAT-IS-REFUSED-BEFORE-THE-PLATFORM-STARTS
check:
  - src/use_cases/version_file.rs::a_load_from_a_newer_format_is_refused_and_the_format_is_read_from_the_runner_copy
  - src/platform/dump_format.rs::the_table_holds_the_measured_platforms
---

# Загрузка из формата новее платформы получает отказ до её запуска

Версия формата читается из файла версий в каталоге исходников до запуска платформы, а если
его там нет — из копии раннера той же пары. Если она новее той, что пишет выбранная
платформа, загрузка отказывает до запуска, и отказ называет версию формата файла и версию,
которую пишет платформа. Если файла версий нет ни в каталоге, ни в памяти раннера или версия
в нём не распознана, проверка не делается, и ответ называет пропуск. Версию, которую пишет
платформа, раннер берёт из той же таблицы замеров, что и выгрузка
(`INV.USE-CASES.A-FOREIGN-FORMAT-VERSION-TURNS-THE-DUMP-FULL`); для платформы вне таблицы
и для чужого агента, чья версия раннеру не видна, сверки нет. Сверка по файлу версий строже
платформы: сама платформа проверяет версию в XML-файлах и каталог, где новее только файл
версий, загрузила бы ([замер](../../../references/1c/confirmed-runtime-measurements.md),
раздел о версии формата файла версий).

Источник: [`platform.html#t22`](../../../docs/site/platform.html#t22),
[`problems.html#p5`](../../../docs/site/problems.html#p5).
