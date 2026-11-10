---
id: INV.PLATFORM.A-PLATFORM-BELOW-THE-MINIMUM-IS-REFUSED
check:
  - src/platform/locator.rs::a_platform_below_the_minimum_is_refused_and_the_minimum_is_accepted
  - src/platform/locator.rs::an_explicit_path_does_not_bypass_the_minimum
  - src/platform/locator.rs::the_minimum_judges_only_a_visible_platform_version
  - src/use_cases/result.rs::an_unsuitable_utility_version_is_an_environment_failure
  - tests/cli_minimum_platform.rs::a_platform_below_the_minimum_is_refused_before_it_starts
  - tests/cli_minimum_platform.rs::platforms_name_an_installation_below_the_minimum_and_do_not_select_it
  - tests/cli_minimum_platform.rs::status_deep_and_a_push_without_memory_name_the_minimum_as_a_reason
---

# Платформу ниже минимума раннер не запускает

Минимальная поддерживаемая версия платформы — **8.3.24** (решение владельца). Утилиту
платформы — `1cv8`, `1cv8c`, `ibcmd`, `webinst` — с версией ниже минимума поиск не выдаёт:
команда отказывает до запуска платформы, отказ называет путь, найденную версию и минимум.
Ниже минимума раннер не проверен, и незнакомая команда бывает не отказом, а зависанием, а
срока у команды нет. Границу подтверждает
[замер](../../../references/1c/confirmed-runtime-measurements.md#раннер-на-83248326-граница-минимальной-версии).

Род отказа — `environment` (`INV.WIRE.A-MISSING-TOOL-IS-AN-ENVIRONMENT-FAILURE`). В цепочке
исполнителей это причина пропуска в квитанции, отказом команды она становится, когда не готов
никто. Превью отказывает так же: оно тоже ищет платформу. `platforms` — диагностика: установку
перечисляет в `installations`, в `selected` отвечает `null`, а причину называет в `warnings`.
`status --deep` тоже диагностика: отвечает успехом, а причину называет в ответе о базе.
`push` без памяти о базе отказывает кодом `untracked_base`, потому что доказать происхождение исходников
нечем: причина стоит в `skipped`, а `push --force` отказывает родом `environment`.

Минимум судит о той установке, которую выбрал поиск: новейшую под маской, по формату исходников
или по `tools.platform.path`. Установка с версией в пути старше той, у которой версии в пути
нет, всё равно считается новее, поэтому выбор падает на неё, и отказ называет её.

Минимум проверяется только там, где версия видна из пути установки. Утилита без каталога версии
проходит: судить не о чем, а запускать платформу ради ответа о версии раннер не станет. Если за
таким путём стоит платформа ниже минимума, команда может зависнуть: 8.3.12 на
`/GetConfigGenerationID` открывает окно и не завершается, отдельного предела для такой платформы
у чтения поколения нет, а срока у команды нет. Чужой
агент (`attached`) и SSH-шлюз автономного сервера утилиты на машине раннера не используют, их
версию раннер не видит и минимум к ним не применяет.
