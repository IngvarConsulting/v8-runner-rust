---
id: INV.CONFIG.BUILDER-KEY-IS-REJECTED
status: active
governs: product
decision: DEC.2026-09-14.BUILDER-KEY-IS-REMOVED
check: [tests/provider_matrix.rs::a_config_with_the_builder_key_is_refused_with_the_replacement_named, src/config/loader.rs::load_config_names_the_replacement_for_a_builder_key_in_the_local_overlay]
scope: [config]
---

# Снятый ключ выбора исполнителя отклоняется

Конфиг с прежним ключом выбора исполнителя не проходит валидацию: молча игнорировать его значило бы делать не то, что написано.
