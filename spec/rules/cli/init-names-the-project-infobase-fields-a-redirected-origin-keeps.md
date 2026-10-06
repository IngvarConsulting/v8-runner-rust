---
id: INV.CLI.INIT-NAMES-THE-PROJECT-INFOBASE-FIELDS-A-REDIRECTED-ORIGIN-KEEPS
check:
  - tests/cli_config_init.rs::init_with_an_infobase_warns_which_project_fields_still_apply_to_the_new_origin
---

# Перенаправление называет поля проектного `infobase:`, которые остаются в `origin`

Когда `init --infobase` перенаправляет `origin`, а проектная секция `infobase:` задаёт
поля помимо `connection`, ответ предупреждает, что загрузчик подмешивает их и в новый
`origin`, называя только имена полей без значений; поля не обнуляются.

Источник: [`cli.html#map`](../../../docs/site/cli.html#map).
