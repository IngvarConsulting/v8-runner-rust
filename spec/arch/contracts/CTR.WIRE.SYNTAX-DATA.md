---
id: CTR.WIRE.SYNTAX-DATA
status: active
governs: product
version: 3
decision: DEC.2026-09-14.RECEIPT-EXPLAINS-PROVIDER-CHOICE
artifact: docs/schemas/command-data/syntax.schema.json
producer: src/domain/syntax.rs
consumers: [cli, mcp, unica]
check: [src/command_data.rs::generated_command_data_schemas_are_current, tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it]
scope: [wire, cli, mcp]
---

# `data` команды `syntax`

Проверка синтаксиса отвечает разобранными замечаниями, а не текстом журнала. У каждого
замечания есть `kind`, и он определяет остальные поля: у модульного — путь, строка и
колонка, у объектного — имя объекта, у EDT — ещё и код проверки. Сводка отделена от
списка, чтобы вызывающий мог решить по числам, не разбирая замечания.

Состав полей замечания закрыт у каждого вида: тело варианта лежит в той же ветке, что
и его `kind`, поэтому поле, не названное здесь, форму валит. До версии 3 состав полей
внутри вида оставался открытым, и добавленное поле проходило молча.

`status` отделяет чистую проверку от найденных замечаний и от упавшего инструмента:
третье — не результат проверки, и путать его с первыми двумя нельзя. Сюда же относится
случай, когда журнал инструмента ожидался и не прочитался: вердикта нет, и чистотой он не
становится. Поле `exit_code` при этом остаётся кодом выхода платформы, поэтому пара
`"status": "tool_failed"` при `"exit_code": 0` читается прямо — инструмент завершился
нулём, а его вердикт прочитать не удалось. Этой же формой
отвечают инструменты MCP `check_syntax_designer_config`, `check_syntax_designer_modules`
и `check_syntax_edt`.

## Пример

```json
{
  "provider": {"selected": "designer", "origin": {"kind": "default"}},
  "status": "issues_found",
  "exit_code": 1,
  "check_name": "designer-config",
  "issues": [
    {
      "kind": "module",
      "path": "src/cf/CommonModules/Демо/Ext/Module.bsl",
      "line": 42,
      "column": 5,
      "severity": "ERROR",
      "message": "Переменная не определена (Значение)"
    }
  ],
  "summary": {
    "errors": 1,
    "warnings": 0,
    "info": 0
  },
  "duration_ms": 321,
  "platform_log_path": "build/logs/platform/syntax_designer-config_0.log"
}
```
