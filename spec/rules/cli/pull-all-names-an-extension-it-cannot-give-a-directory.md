---
id: INV.CLI.PULL-ALL-NAMES-AN-EXTENSION-IT-CANNOT-GIVE-A-DIRECTORY
check:
  - src/platform/extension_inventory.rs::a_windows_device_name_is_an_identifier_but_not_a_directory
  - src/use_cases/dump_config/all.rs::a_windows_device_name_is_named_not_declared
  - tests/cli_pull_all.rs::an_extension_named_like_a_windows_device_is_named_and_the_rest_is_pulled
  - tests/cli_extensions.rs::extensions_info_accepts_a_windows_device_name_as_an_identifier
---

# Расширению с именем устройства Windows набор не объявляется

Имя устройства Windows (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`, без учёта
регистра) — корректный идентификатор 1С, но каталогом `src/ext/<Name>` в Windows ему не
стать. Расширению без набора с таким именем `pull --all` набор не объявляет и не выгружает
его, а называет в `data.not_declared` с причиной и советом объявить набор вручную под другим
путём; остальные наборы выгружаются и объявляются. Команды, которые каталога не заводят,
такое имя принимают.
