---
id: INV.PLATFORM.PLATFORM-TOOLS-ARE-FOUND-BY-VERSION-MASK
check:
  - tests/cli_launch.rs::launch_json_exposes_platform_resolution_metadata
  - src/platform/locator.rs::strict_prefix_selects_highest_across_direct_and_versioned_candidates
  - src/platform/locator.rs::version_only_resolution_uses_default_roots_and_path_with_version_filter
---

# Утилиты платформы ищет один locator по маске версии

Пути к `1cv8`, `1cv8c`, `ibcmd` и EDT CLI ищет одно место. Версия задаётся точно,
`major.minor.patch` или `major.minor`; из подошедших под маску берётся максимальная
установленная, а без указанной версии — максимальная найденная.

`tools.platform.path` только сужает область поиска. Сценарии путей к двоичным файлам не
собирают, а найденная установка называется в квитанции.
