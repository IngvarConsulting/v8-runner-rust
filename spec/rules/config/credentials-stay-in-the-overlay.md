---
id: INV.CONFIG.CREDENTIALS-STAY-IN-THE-OVERLAY
check:
  - tests/cli_bootstrap.rs::bootstrap_json_success_keeps_credentials_in_local_overlay_only
  - tests/cli_config_init.rs::init_in_a_new_worktree_redirects_the_copied_origin_and_keeps_it_as_upstream
---

# Учётные данные попадают только в локальный слой

Команда заведения проекта пишет учётные данные исключительно в некоммитируемый локальный слой.
