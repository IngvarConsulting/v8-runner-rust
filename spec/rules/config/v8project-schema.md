---
id: CTR.CONFIG.V8PROJECT-SCHEMA
version: 12
artifact: docs/schemas/v8project.schema.json
check:
  - src/config/schema.rs::generated_schema_artifacts_are_current
  - src/config/schema.rs::the_shared_key_is_refused_as_unknown_with_a_hint
---

# Опубликованные схемы конфигурации

Форма `v8project.yaml` и его локального слоя опубликована JSON-схемами — своей у
каждого файла. Базы объявляет местный слой картой `infobases`; проектный файл базы не
называет, а прежний ключ `infobase` помечен в каждой схеме как `deprecated` на один цикл
выпуска. Секция базы держит секцию `cluster`: адрес сервера администрирования `ras` и
уровни администраторов над пользователем базы — `user`/`password` кластера и `agent` с
`address`, `user`, `password` центрального сервера; все ключи необязательны. Схемы
порождаются из типизированной модели и обязаны совпадать с ней: расхождение
валит проверку, а не обнаруживается в редакторе пользователя. Артефакт обновляется
командой `UPDATE_CONFIG_SCHEMAS=1 cargo test generated_schema_artifacts_are_current`.

**Что изменила версия 12.** Секция базы местного слоя больше не принимает `shared`: общих баз
нет (`INV.CONFIG.THE-SHARED-KEY-IS-REFUSED-WITH-A-HINT`). Несовместимо с 0.13.0.

**Что изменила версия 11.** Ключ `standalone.gate` необязателен: автономный сервер
объявлен строкой прямого шлюза в `connection`, SSH-шлюзом или строкой и шлюзом вместе
(`INV.CONFIG.A-STANDALONE-TARGET-ACCEPTS-EITHER-GATE-KEY`). Описание `standalone.exchange`
называет канал нужным SSH-шлюзу без строки.

**Что изменила версия 10.** Секция базы местного слоя принимала `shared` — согласие этой
рабочей копии делить файловую базу с остальными держателями; версия 12 его убрала.

**Что изменила версия 9.** Секция `push` больше не описывает ключ `partialLoadThreshold`:
схема его не допускает, а загрузчик отвергает по имени. Описания `cluster.ras` и
`cluster.agent.address` называют хост именем или IPv4: адрес IPv6 отвергается.

## Пример

```yaml
workPath: build
format: DESIGNER
providers:
  push: ibcmd
source-set:
  - name: main
    type: CONFIGURATION
    path: src/cf
```
