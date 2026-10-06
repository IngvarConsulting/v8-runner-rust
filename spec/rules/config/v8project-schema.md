---
id: CTR.CONFIG.V8PROJECT-SCHEMA
version: 10
artifact: docs/schemas/v8project.schema.json
check:
  - src/config/schema.rs::generated_schema_artifacts_are_current
  - src/config/schema.rs::the_local_layer_declares_consent_to_share_a_base
  - src/config/schema.rs::the_project_file_refuses_consent_to_share_a_base
---

# Опубликованные схемы конфигурации

Форма `v8project.yaml` и его локального слоя опубликована JSON-схемами — своей у
каждого файла. Базы объявляет местный слой картой `infobases`; проектный файл базы не
называет, а прежний ключ `infobase` помечен в каждой схеме как `deprecated` на один цикл
выпуска. Секция базы держит секцию `cluster`: адрес сервера администрирования `ras` и
уровни администраторов над пользователем базы — `user`/`password` кластера и `agent` с
`address`, `user`, `password` центрального сервера; все ключи необязательны. Согласие
рабочей копии делить файловую базу — ключ `shared` секции базы — описывает только схема
местного слоя: проектный файл, в том числе прежняя секция `infobase:`, ключ отвергает по
имени, потому что согласие одной копии не коммитят. Схемы
порождаются из типизированной модели и обязаны совпадать с ней: расхождение
валит проверку, а не обнаруживается в редакторе пользователя. Артефакт обновляется
командой `UPDATE_CONFIG_SCHEMAS=1 cargo test generated_schema_artifacts_are_current`.

**Что изменила версия 10.** Секция базы местного слоя принимает `shared` — согласие этой
рабочей копии делить файловую базу с остальными держателями
(`INV.USE-CASES.A-BASE-IS-SHARED-BY-CONSENT-OF-EVERY-COPY`); схема проектного файла его не
описывает.

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
