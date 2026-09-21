---
id: INV.USE-CASES.A-MISSING-VERSION-FILE-IS-KNOWN-BEFORE-THE-PLATFORM-STARTS
status: planned
governs: product
decision: DEC.2026-09-21.THE-VERSION-FILE-BELONGS-TO-THE-RUNNER
check: null
scope: [use-cases, platform]
---

# Отсутствие файла версий известно до запуска платформы

Выгрузка по изменившемуся при отсутствующем у раннера файле версий или при чужой версии
его формата переводится в полную до запуска платформы: `-update` в аргументах не
появляется, а ответ называет причину.
