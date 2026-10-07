---
id: INV.CONFIG.THE-SHARED-KEY-IS-REFUSED-WITH-A-HINT
check:
  - src/config/schema.rs::the_shared_key_is_refused_as_unknown_with_a_hint
  - tests/cli_infobases.rs::a_config_with_the_shared_key_is_refused_with_a_hint
---

# Ключ `shared` отклоняется как незнакомый с подсказкой

Ключ `shared` секции базы — в карте `infobases` и в прежней секции `infobase` местного слоя,
и в прежней секции `infobase` проектного файла — загрузчик отклоняет ошибкой незнакомого
ключа. Ошибка называет секцию и файл, просит убрать `shared` и говорит, что запись в базу
другой рабочей копии теперь идёт с предупреждением. Ни одна схема конфигурации ключа не
описывает.

Решение владельца от 07.10.2026 (#437): общих баз нет, ключ удаляется сразу, переходного
чтения нет. Несовместимо с 0.13.0, где ключ принимал местный слой.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
