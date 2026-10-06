---
id: INV.USE-CASES.A-CLUSTER-DESIGNER-GENERATION-IS-READ-LIKE-A-FILE-ONE
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/184
---

# Ответ Конфигуратора о поколении у базы в кластере разбирается как у файловой

Конфигуратор с базой в кластере разбирает ответ `/GetConfigGenerationID` так же, как с
файловой базой. Нераспознанный ответ — отсутствие ответа: сверки нет, отказа нет. Формат
ответа у базы в кластере не замерен
([замер](../../../references/1c/confirmed-runtime-measurements.md)).
