---
id: INV.PLATFORM.PLATFORM-TOOLS-ARE-FOUND-BY-VERSION-MASK
check:
  - tests/cli_launch.rs::launch_json_exposes_platform_resolution_metadata
  - src/platform/locator.rs::strict_prefix_selects_highest_across_direct_and_versioned_candidates
  - src/platform/locator.rs::version_only_resolution_uses_default_roots_and_path_with_version_filter
  - src/platform/locator.rs::the_source_format_chooses_its_release_over_the_newest
  - src/platform/locator.rs::a_release_of_the_source_format_does_not_borrow_a_component_from_another
  - src/platform/locator.rs::without_the_release_of_the_source_format_the_newest_is_chosen
  - src/platform/locator.rs::an_explicit_version_outranks_the_source_format
  - tests/cli_platforms.rs::without_a_version_the_sources_format_chooses_the_release
---

# Утилиты платформы ищет один locator по маске версии

Пути к `1cv8`, `1cv8c`, `ibcmd` и EDT CLI ищет одно место. Версия задаётся точно,
`major.minor.patch` или `major.minor`; из подошедших под маску берётся максимальная
установленная.

Когда версию не задали ни маской, ни путём, её выбирает формат исходников проекта формата
Конфигуратора — атрибут `version` корня `Configuration.xml` основного набора: берётся
максимальная установка выпуска, который пишет этот формат по таблице замеров. Выпуск
закрепляется за всеми утилитами: компонента, которого в нём нет, не ищут в другом выпуске.
Если формат раннеру не известен или его выпуск не установлен, берётся максимальная
найденная; о расхождении `platforms` предупреждает в `warnings`, остальные команды — в
журнале. Выпуск вне таблицы замеров формат не
выбирает, даже если пишет тот же формат.

`tools.platform.path` только сужает область поиска. Сценарии путей к двоичным файлам не
собирают, а найденная установка называется в квитанции.
