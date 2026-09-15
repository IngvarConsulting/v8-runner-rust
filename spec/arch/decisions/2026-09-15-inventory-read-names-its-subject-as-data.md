---
id: DEC.2026-09-15.INVENTORY-READ-NAMES-ITS-SUBJECT-AS-DATA
status: active
governs: product
realized: tests/cli_extensions.rs::extensions_info_preview_names_the_requested_extension_as_data
supersedes: []
superseded-by: null
establishes: [CTR.WIRE.EXTENSIONS-INVENTORY-DATA]
changes: [CTR.WIRE.EXTENSIONS-INVENTORY-DATA]
---

# Чтение состава называет свой предмет данными

**Решение.** Ответ `extensions list` и `extensions info` несёт поле `requested`:
`{"kind": "all"}` или `{"kind": "named", "name": …}`. Поле есть и в превью, и в
ответе после запуска платформы. Строка `plan` остаётся: она для человека и ничего
не решает.

**Почему.** Превью изменения состава уже называет предмет данными — `target` и
`action` в `steps`, — а превью чтения называло его только словами внутри `plan`.
Вызывающий, который сверяет превью со своим запросом, вынужден был разбирать
фразу, а по `DEC.2026-09-12.TOOL-PROSE-NEVER-DECIDES` формулировка входом решения
быть не может. Теперь у чтения и изменения одна дисциплина: предмет — поле.

**Не затрагивает.** Состав `extensions` и его порядок; квитанцию `provider`;
текст `plan` — он по-прежнему не содержит секретов из строки соединения.
