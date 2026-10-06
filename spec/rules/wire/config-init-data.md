---
id: CTR.WIRE.CONFIG-INIT-DATA
version: 3
artifact: docs/schemas/command-data/init.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/cli_config_init.rs::config_init_uses_json_envelope_and_output_override
  - tests/cli_config_init.rs::init_in_a_declared_project_leaves_the_project_file_and_writes_the_local_layer
  - tests/cli_config_init.rs::init_in_a_new_worktree_redirects_the_copied_origin_and_keeps_it_as_upstream
---

# `data` команды `init`

Форма отвечает на вопрос, что `init` записал. Различитель `kind` называет вариант.

`kind: "project"` — записан проектный файл: каталог был без него или его переписал
`--force`. Найденные наборы исходников перечислены тем же составом полей, каким они
легли в `v8project.yaml`, а `overwritten` говорит, был ли затёрт существовавший файл.
Поля `path`, `format`, `platform_version`, `source_sets`, `overwritten` и `warnings`
есть только у этого варианта.

`kind: "local"` — проектный файл уже был и остался нетронутым, записан только местный
слой. Вариант несёт `local_path`, `gitignore_path` и `origin`.

`origin` у каждого варианта говорит, что стало с `infobases.origin` местного слоя:
`change` — `declared` (адрес записан туда, где его не было), `unchanged` (секция не
менялась) или `redirected` (секция получила новый адрес, прежняя сохранена под именем
`upstream`); `connection` — адрес в `origin` после команды, его нет у автономного
сервера; `replaced` — заменённый адрес, он есть только у `redirected` и только если у
прежней секции был адрес. Пароль и имя пользователя внутри адреса замаскированы,
учётные данные секций в форму не попадают.

**Что изменила версия 3.** Форма стала размеченным перечислением: появились
различитель `kind` и вариант `local`, которого прежде не было, — `init` в объявленном
проекте отказывал. Прежние поля перешли в вариант `project` без изменений. Каждый вариант
получил обязательное `origin`.

## Пример

```json
{
  "kind": "local",
  "ok": true,
  "local_path": "/work/erp-task/v8project.local.yaml",
  "gitignore_path": "/work/erp-task/.gitignore",
  "origin": {
    "change": "redirected",
    "connection": "File=build/ib",
    "replaced": "Srvr=srv;Ref=erp;Pwd=***"
  },
  "duration_ms": 9
}
```
