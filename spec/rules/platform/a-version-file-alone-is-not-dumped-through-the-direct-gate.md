---
id: INV.PLATFORM.A-VERSION-FILE-ALONE-IS-NOT-DUMPED-THROUGH-THE-DIRECT-GATE
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/184
---

# Один файл версий через прямой шлюз не выгружается

Выгрузку одного файла версий (`-configDumpInfoOnly`) Конфигуратор по прямому шлюзу
автономного сервера не получает: ответа о поколении у него нет
(`INV.USE-CASES.A-CLUSTER-DESIGNER-GENERATION-IS-READ-LIKE-A-FILE-ONE`), а без него совпадение
каталога и базы не доказано. Ответ `-configDumpInfoOnly` за прямым шлюзом не замерен.
