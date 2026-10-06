---
id: INV.CLI.INIT-IN-A-DECLARED-PROJECT-LEAVES-THE-PROJECT-FILE
check:
  - tests/cli_config_init.rs::init_in_a_declared_project_leaves_the_project_file_and_writes_the_local_layer
  - tests/cli_config_init.rs::init_in_a_new_worktree_redirects_the_copied_origin_and_keeps_it_as_upstream
  - tests/cli_config_init.rs::init_with_the_address_already_in_origin_changes_nothing
  - src/use_cases/config_init.rs::a_declared_project_redirects_origin_and_keeps_the_previous_section_as_upstream
  - src/use_cases/config_init.rs::force_rewrites_the_project_file_and_redirects_origin_as_without_it
  - tests/cli_config_init.rs::init_over_the_infobase_synonym_in_the_project_file_keeps_the_declared_origin
  - tests/cli_config_init.rs::init_with_an_infobase_redirects_the_origin_declared_by_the_infobase_synonym
  - tests/cli_config_init.rs::init_refuses_to_redirect_the_infobase_synonym_over_an_existing_upstream
  - tests/cli_config_init.rs::init_with_an_infobase_warns_which_project_fields_still_apply_to_the_new_origin
  - src/use_cases/config_init.rs::the_infobase_synonym_of_the_project_file_needs_no_local_layer
  - src/use_cases/config_init.rs::force_carries_the_infobase_synonym_of_the_project_file_into_the_local_layer
---

# `init` в готовом проекте не трогает проектный файл

В проекте, где `v8project.yaml` уже есть, `init` без `--force` его не трогает. Без
`--infobase` он объявляет `origin` базой `File=build/ib`, если `origin` ещё нет. С
`--infobase <адрес>` он отдаёт `origin` новый адрес, а прежнюю секцию сохраняет под именем
`upstream` вместе с учётными данными; ответ называет заменённый адрес. Адрес, который уже
стоит в `origin`, ничего не меняет. `--force` переписывает только проектный файл: с местным
слоем `init` поступает так же, как без него.

Прежний ключ `infobase:` в проектном файле объявляет `origin`: `init` сливает его с
местным `infobases.origin` по полям, как загрузчик, и решает по действующей секции. Без
`--infobase` ответ `unchanged`, если действующий адрес есть, и `declared`, если `origin`
не объявлен ни в одном слое. С `--infobase` в `upstream` местного слоя уходит действующая
секция целиком, `origin` местного слоя получает новый адрес, а занятый `upstream` — отказ.
Проектный файл остаётся как был, предупреждение о синониме сохраняется. Поля проектной
секции, кроме адреса, загрузчик подмешивает и в новый `origin`; если они есть, ответ
перенаправления предупреждает об этом, называя только имена полей, и советует перенести
`infobase:` в местный слой. Поля не обнуляются. При `--force`
проектный файл переписывается без этого ключа, поэтому действующая секция переезжает в
местный слой и `origin` остаётся тем же.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies),
[`cli.html#map`](../../../docs/site/cli.html#map).
