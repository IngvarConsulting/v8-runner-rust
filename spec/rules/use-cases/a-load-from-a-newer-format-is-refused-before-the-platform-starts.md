---
id: INV.USE-CASES.A-LOAD-FROM-A-NEWER-FORMAT-IS-REFUSED-BEFORE-THE-PLATFORM-STARTS
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/214
---

# Загрузка из формата новее платформы получает отказ до её запуска

Версия формата читается из файла версий в каталоге исходников до запуска платформы. Если
она новее той, что пишет выбранная платформа, загрузка отказывает до запуска, и отказ
называет версию формата файла и версию, которую пишет платформа. Если файла версий нет ни в каталоге, ни в памяти раннера, проверка не делается, и ответ называет пропуск. Выгрузку при чужой версии держит
`INV.USE-CASES.A-MISSING-VERSION-FILE-IS-KNOWN-BEFORE-THE-PLATFORM-STARTS`.

Источник: [`platform.html#t22`](../../../docs/site/platform.html#t22),
[`problems.html#p5`](../../../docs/site/problems.html#p5).
