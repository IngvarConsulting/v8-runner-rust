---
id: INV.USE-CASES.THE-VERSION-FILE-STAYS-OUT-OF-VERSION-CONTROL
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/162
---

# Файлу версий нет места в системе контроля версий

`ConfigDumpInfo.xml` — опись состояния одной базы, а не исходный код. `init` и `clone` пишут
его в игнор вместе с местным слоем, а найдя файл в индексе, раннер останавливается.
