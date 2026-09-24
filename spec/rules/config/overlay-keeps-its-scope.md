---
id: INV.CONFIG.OVERLAY-KEEPS-ITS-SCOPE
check: [tests/cli_bootstrap.rs::unsupported_local_overlay_shape_is_rejected_in_json_mode]
---

# Локальный слой за пределами своей области отклоняется

Ключ, которого локальному слою иметь не положено, приводит к отказу валидации, а не к молчаливому применению.
