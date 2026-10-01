---
id: INV.CONFIG.CREDENTIALS-STAY-IN-THE-OVERLAY
check: [tests/cli_bootstrap.rs::bootstrap_json_success_keeps_credentials_in_local_overlay_only]
---

# Учётные данные попадают только в локальный слой

Команда заведения проекта пишет учётные данные исключительно в некоммитируемый локальный слой.
