---
id: INV.USE-CASES.AN-UNESTABLISHED-COMPATIBILITY-IS-A-PLATFORM-FAILURE
check: [src/use_cases/load_artifact.rs::an_unestablished_compatibility_answers_a_platform_failure]
---

# Неустановленная совместимость — сбой платформы

Совместимость спросили и не установили — отказ несёт род `platform`: не открылась база или
не прочитался перечень расширений, а запрос верен. Род `validation` говорит о запросе и
здесь не годится. Изменений такой отказ не разрешает.

Отказ по `NotEstablished` строится как `AppError::Platform`
(`src/use_cases/load_artifact.rs`).
