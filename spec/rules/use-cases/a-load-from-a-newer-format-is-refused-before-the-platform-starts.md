---
id: INV.USE-CASES.A-LOAD-FROM-A-NEWER-FORMAT-IS-REFUSED-BEFORE-THE-PLATFORM-STARTS
check:
  - tests/cli_pull_memory.rs::a_load_from_a_newer_format_is_refused_before_the_platform_starts
  - src/use_cases/version_file.rs::the_load_format_is_read_from_the_runner_copy_when_the_directory_has_none
  - src/platform/dump_format.rs::only_a_documented_platform_has_a_known_format
---

# Загрузка из формата новее платформы получает отказ до её запуска

Версия формата читается из файла версий в каталоге исходников до запуска платформы. Если
она новее той, что пишет выбранная платформа, загрузка отказывает до запуска, и отказ
называет версию формата файла и версию, которую пишет платформа. Если файла версий нет ни в каталоге, ни в памяти раннера, проверка не делается, и ответ называет пропуск. Версию,
которую пишет платформа, раннер берёт из той же таблицы, что и выгрузка; для платформы вне
таблицы и для чужого агента, чья версия раннеру не видна, сверки нет. Выгрузку при чужой
версии держит
`INV.USE-CASES.A-MISSING-VERSION-FILE-IS-KNOWN-BEFORE-THE-PLATFORM-STARTS`.

Источник: [`platform.html#t22`](../../../docs/site/platform.html#t22),
[`problems.html#p5`](../../../docs/site/problems.html#p5).
