---
id: INV.CONFIG.UNSAFE-COMBINATIONS-ARE-REJECTED-BEFORE-DISPATCH
check: [tests/contract_config_boundary.rs::an_unsupported_combination_is_refused_before_any_utility_runs]
---

# Неподдержанное сочетание отклоняется до вызова платформы

Валидация конфига заканчивается отказом раньше, чем запускается любая утилита: цена ошибки не должна включать частично сделанную работу.
