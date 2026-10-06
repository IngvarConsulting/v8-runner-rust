---
id: CTR.CONFIG.V8PROJECT-SCHEMA
version: 9
artifact: docs/schemas/v8project.schema.json
check: [src/config/schema.rs::generated_schema_artifacts_are_current]
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

**Что изменила версия 9.** Секция `push` больше не описывает ключ `partialLoadThreshold`:
схема его не допускает, а загрузчик отвергает по имени. Описания `cluster.ras` и
`cluster.agent.address` называют хост именем или IPv4: адрес IPv6 отвергается.

## Пример

```yaml
workPath: build
format: DESIGNER
providers:
  build: ibcmd
source-set:
  - name: main
    type: CONFIGURATION
    path: src/cf
```
