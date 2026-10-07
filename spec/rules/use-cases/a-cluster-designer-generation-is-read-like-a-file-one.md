---
id: INV.USE-CASES.A-CLUSTER-DESIGNER-GENERATION-IS-READ-LIKE-A-FILE-ONE
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/184
---

# Ответ Конфигуратора о поколении у базы в кластере разбирается как у файловой

Конфигуратор с базой в кластере и с автономным сервером по прямому шлюзу разбирает ответ
`/GetConfigGenerationID` так же, как с файловой базой. Нераспознанный ответ — отсутствие
ответа: сверки нет, отказа нет. Формат ответа у серверной базы не замерен ни в кластере, ни
за прямым шлюзом
([замер](../../../references/1c/confirmed-runtime-measurements.md)).
