---
id: INV.CLI.INIT-READS-THE-PROJECT-INFOBASE-SYNONYM-AS-THE-DECLARED-ORIGIN
check:
  - tests/cli_config_init.rs::init_over_the_infobase_synonym_in_the_project_file_keeps_the_declared_origin
  - tests/cli_config_init.rs::init_with_an_infobase_redirects_the_origin_declared_by_the_infobase_synonym
  - tests/cli_config_init.rs::init_refuses_to_redirect_the_infobase_synonym_over_an_existing_upstream
  - src/use_cases/config_init.rs::the_infobase_synonym_of_the_project_file_needs_no_local_layer
---

# `init` считает прежний `infobase:` проектного файла объявленным `origin`

Прежний ключ `infobase:` в `v8project.yaml` для `init` — объявленный `origin`: решение
об `origin` принимается по секции, слитой с местным `infobases.origin` по полям так же, как
её сливает загрузчик, и при перенаправлении в `upstream` местного слоя уходит именно она, а
проектный файл остаётся как был и предупреждение о синониме сохраняется.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies),
[`cli.html#map`](../../../docs/site/cli.html#map).
