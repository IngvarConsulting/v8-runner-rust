---
id: INV.CONFIG.A-STANDALONE-TARGET-ACCEPTS-EITHER-GATE-KEY
check:
  - src/config/validate.rs::a_standalone_target_takes_either_way_and_refuses_neither
  - tests/cli_standalone_direct_gate.rs::a_standalone_server_with_only_the_direct_gate_is_served_by_the_designer
  - tests/cli_agent_standalone.rs::a_direct_gate_address_next_to_the_ssh_gate_puts_the_designer_first
---

# Автономной цели достаточно строки прямого шлюза или SSH-шлюза

Секция базы с объявленной секцией `standalone` проходит валидацию со строкой прямого шлюза
в `connection` без `standalone.gate`, с одним `standalone.gate` и со строкой и шлюзом вместе;
ни один ключ не обязателен при наличии другого. Секция без строки и без шлюза — ошибка
валидации, которая называет `infobase.connection` и `infobase.standalone.gate`. Строку
рядом с секцией загрузчик принимает без предупреждений.
