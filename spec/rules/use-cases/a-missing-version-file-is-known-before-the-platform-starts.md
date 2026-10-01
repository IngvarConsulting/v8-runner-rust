---
id: INV.USE-CASES.A-MISSING-VERSION-FILE-IS-KNOWN-BEFORE-THE-PLATFORM-STARTS
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/214
---

# Отсутствие файла версий известно до запуска платформы

Выгрузка по изменившемуся при отсутствующем у раннера файле версий или при чужой версии
его формата переводится в полную до запуска платформы: `-update` в аргументах не
появляется, а ответ называет причину.
