---
id: INV.DOCS.A-SYMBOL-AND-ITS-PATH-SPELL-EACH-OTHER
status: active
governs: process
decision: DEC.2026-09-16.A-SYMBOL-AND-ITS-PATH-SPELL-EACH-OTHER
check: tests/arch_registry.rs::a_symbol_and_its_path_spell_each_other
scope: [docs]
---

# Символ записи и путь к ней восстанавливают друг друга

`id` совпадает с символом, который диктует имя файла: у решения `<дата>-<имя>.md` это
`DEC.<дата>.<ИМЯ>`, у правила — имя файла без расширения. Префикс `id` совпадает с видом
каталога: `DEC.` в `decisions/`, `INV.` в `invariants/`, `CTR.` в `contracts/`. Решение с
именем файла вне этой формы реестром отклоняется.
