---
id: INV.CONFIG.RELATIVE-PATHS-RESOLVE-FROM-THE-CONFIG-DIRECTORY
check:
  - tests/cli_build.rs::ibcmd_push_receives_config_relative_paths_resolved_from_the_config_directory
  - tests/cli_load.rs::upload_with_ibcmd_push_receives_config_relative_paths_resolved_from_the_config_directory
  - tests/cli_test.rs::vanessa_resolves_a_relative_epf_path_from_a_nested_config_directory
  - src/config/loader.rs::load_config_absolutizes_relative_core_paths_from_config_dir
  - src/config/loader.rs::tool_paths_resolve_from_the_config_directory
---

# Относительный путь конфига разрешается от каталога основного конфига

Относительный путь из `v8project.yaml` и местного слоя считается от каталога основного
`v8project.yaml`, а не от рабочего каталога процесса: та же строка даёт тот же путь в
проверке конфигурации, в argv утилит платформы и в порождённых параметрах Vanessa,
откуда бы раннер ни запустили.

Исключения:

- `tools.edt_cli.path` без разделителя — имя или подсказка версии, а не путь: его
  разбирает локатор EDT. Путь с разделителем подчиняется правилу.
- Пути внутри шаблона `tests.va.params_path` раннер не разрешает: их разрешает сама
  Vanessa от `WorkspaceRoot`, который раннер ставит в каталог основного конфига, если
  шаблон его не задаёт.
- На Windows путь с буквой диска без корня (`C:foo`) относительным не считается: он
  заменяет каталог конфига целиком и указывает в текущий каталог своего диска.
