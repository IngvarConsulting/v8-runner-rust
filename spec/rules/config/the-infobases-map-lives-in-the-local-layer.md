---
id: INV.CONFIG.THE-INFOBASES-MAP-LIVES-IN-THE-LOCAL-LAYER
check:
  - tests/cli_infobases.rs::the_map_is_refused_in_the_project_file
  - tests/cli_config_init.rs::init_refuses_the_infobases_map_in_the_project_file_as_the_loader_does
---

# Карта `infobases` живёт только в местном слое

Карта `infobases` в `v8project.yaml` — отказ загрузчика, который называет местный слой;
`init` в таком проекте отказывает тем же отказом и ничего не записывает.
