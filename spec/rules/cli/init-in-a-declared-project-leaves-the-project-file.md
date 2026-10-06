---
id: INV.CLI.INIT-IN-A-DECLARED-PROJECT-LEAVES-THE-PROJECT-FILE
check:
  - tests/cli_config_init.rs::init_in_a_declared_project_leaves_the_project_file_and_writes_the_local_layer
  - tests/cli_config_init.rs::init_in_a_new_worktree_redirects_the_copied_origin_and_keeps_it_as_upstream
  - tests/cli_config_init.rs::init_with_the_address_already_in_origin_changes_nothing
  - src/use_cases/config_init.rs::a_declared_project_redirects_origin_and_keeps_the_previous_section_as_upstream
  - src/use_cases/config_init.rs::force_rewrites_the_project_file_and_redirects_origin_as_without_it
---

# `init` в готовом проекте не трогает проектный файл

В проекте, где `v8project.yaml` уже есть, `init` без `--force` его не трогает. Без
`--infobase` он объявляет `origin` базой `File=build/ib`, если `origin` ещё нет. С
`--infobase <адрес>` он отдаёт `origin` новый адрес, а прежнюю секцию сохраняет под именем
`upstream` вместе с учётными данными; ответ называет заменённый адрес. Адрес, который уже
стоит в `origin`, ничего не меняет. `--force` переписывает только проектный файл: с местным
слоем `init` поступает так же, как без него.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies),
[`cli.html#map`](../../../docs/site/cli.html#map).
