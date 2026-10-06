---
id: INV.CLI.AN-EXISTING-UPSTREAM-IS-NOT-OVERWRITTEN
check:
  - tests/cli_config_init.rs::init_refuses_to_redirect_origin_over_an_existing_upstream
  - src/use_cases/config_init.rs::an_existing_upstream_refuses_the_redirect_and_names_origin_and_upstream
---

# Существующий `upstream` не перезаписывается

Если в местном слое уже есть `upstream`, `init --infobase` с новым адресом отказывает и
называет `origin` и `upstream`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies),
[`cli.html#map`](../../../docs/site/cli.html#map).
