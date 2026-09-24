---
id: INV.USE-CASES.AN-UNESTABLISHED-COMPATIBILITY-IS-A-PLATFORM-FAILURE
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/285
---

# Неустановленная совместимость — сбой платформы

Совместимость спросили и не установили — отказ несёт род `platform`: не открылась база или
не прочитался перечень расширений, а запрос верен. Род `validation` говорит о запросе и
здесь не годится. Изменений такой отказ не разрешает.

Сегодня отказ по `NotEstablished` строится как `AppError::Validation`
(`src/use_cases/load_artifact.rs`).
