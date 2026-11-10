---
id: INV.PLATFORM.A-MISSING-RELEASE-OF-THE-SOURCES-FORMAT-IS-REFUSED
check:
  - src/platform/locator.rs::a_missing_release_of_the_sources_format_is_refused_with_the_inventory
  - src/platform/locator.rs::the_sources_setting_chooses_the_installed_release
  - src/platform/locator.rs::an_unknown_sources_format_is_refused
  - src/platform/locator.rs::the_sources_release_is_enforced_inside_the_path
  - src/platform/locator.rs::a_path_installation_counts_for_the_sources_release
  - src/platform/locator.rs::the_platform_version_setting_is_a_mask_or_sources
  - src/config/validate.rs::the_sources_platform_version_needs_a_designer_main_set
  - src/use_cases/provider_selection.rs::a_missing_release_of_the_sources_format_reaches_the_caller_typed
  - tests/cli_platform_release.rs::every_platform_command_refuses_without_the_release_of_the_sources_format
  - tests/cli_platform_release.rs::a_foreign_generation_record_answers_the_release_refusal_not_non_fast_forward
  - tests/cli_platform_release.rs::platforms_name_the_release_refusal_in_the_warnings
  - tests/cli_platform_release.rs::an_installed_release_of_the_sources_format_is_chosen
---

# При `tools.platform.version: sources` нет выпуска формата исходников — отказ

Значение `sources` ключа `tools.platform.version` выбирает выпуск платформы по формату
исходников основного набора — атрибуту `version` корня его `Configuration.xml` — по той же
таблице замеров, что и выбор по умолчанию (`INV.PLATFORM.PLATFORM-TOOLS-ARE-FOUND-BY-VERSION-MASK`):
берётся новейшая сборка этого выпуска, и выпуск закреплён за всеми утилитами платформы
(`1cv8`, `1cv8c`, `ibcmd`, `ragent`, `ibsrv`, `webinst`). В отличие от умолчания отката на
новейшую установку нет. Значение разбирается одним местом вместе с маской
(`PlatformVersionSetting`); другой код строку `sources` не сравнивает.

Отказ — до запуска платформы, в поиске утилиты (`Locator::locate_platform`), раньше выбора
кандидата и проверки минимума, когда:

- выпуска, который пишет формат, среди установок нет;
- формата нет в таблице замеров;
- версия формата в корне не найдена или файл не читается.

Выпуск вне таблицы, даже пишущий тот же формат, не засчитывается, и текст отказа это
говорит. Код отказа — `platform_release_missing`, род `environment`, выход 2; код шага в
`data.execution.errors[]` остаётся `environment_unavailable`. Поле `platform_release`
отказа несёт формат исходников (версия, файл, выпуск) и опись установок — тех самых, по
которым отказ решён: под `tools.platform.path`, если он задан, иначе в корнях по умолчанию
и в `PATH`. По MCP отказ приходит кодом `runtime_failure` с тем же полем.

С `tools.platform.path` выпуск обязателен и под путём, при любом `strict`; `strict` при
`sources` не действует. Выпуск установлен, а нужного компонента в нём нет (например, только
`1cv8c`) — это не отказ выпуска, а прежний `environment_unavailable` с описью
(`INV.PLATFORM.A-MISSING-COMPONENT-NAMES-WHAT-IS-INSTALLED`). `Configuration.xml` основного
набора ещё нет — проект до первой выгрузки, — и выбор ничем не ограничен.

Настройки отвергаются до команды, если `sources` задан у проекта формата EDT или у
проекта без основного набора: проверка ключа идёт во всех путях проверки настроек, где
команда может искать платформу.

Отказ доходит до ответа как есть, от любого вызывающего: выбор исполнителя, когда не готов
никто, отвечает им, а не перечнем пропущенных, а сверка поколения инструментом записи не
превращает его в «инструмент не ответил» и отказ `non_fast_forward`. `platforms` называет
его в `warnings` и ставит `null` в `selected`.
