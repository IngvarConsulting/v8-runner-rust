---
id: CTR.WIRE.SYNTAX-DATA
version: 5
artifact: docs/schemas/command-data/check.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - tests/mcp_stdio.rs::mcp_stdio_tools_answer_in_the_forms_of_their_commands
  - tests/mcp_stdio.rs::mcp_stdio_the_live_edt_check_answers_in_the_form_of_check
---
# `data` команды `check`

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

Четвёртое значение `status` — `planned`: так отвечает превью. Остальные три — приговоры
конфигурации, а превью конфигурацию не смотрело, поэтому `clean` из него был бы
приговором выдуманным. `exit_code` при `planned` равен `-1`: кода выхода не наблюдалось,
потому что платформа не запускалась. Поле остаётся обязательным и числовым — вызывающий,
читавший его раньше, читает его и теперь. `platform_log_path` превью не называет: каталог
журналов платформы не создаётся, и файла не будет. Через MCP `planned` не приходит:
превью в опубликованной поверхности сервера не предлагается.

`provider_dispatched` отделяет запланированное от выполненного: `false` ровно тогда, когда
прогон остановился на превью. Ветка EDT квитанции `provider` не несёт ни в превью, ни в
боевом прогоне — выбирать там не из чего, EDT CLI ищется напрямую.

Необязательное поле `message` называет предмет словами: превью говорит им, что было бы
выполнено — команду платформы с режимами и найденную утилиту.

Поле `check_name` называет, чем платформа выполнила проверку, и набор его значений
закрыт: `designer-config` или `edt`. Прежнее `designer-modules` исчезло вместе с отдельным
путём `/CheckModules` — режимы проверки модулей выполняет `/CheckConfig`, и вызов прежнего
имени отвечает под новым. Это же значение попадает в имя файла журнала платформы, поэтому у проверки модулей он
теперь `syntax_designer-config_*.log`; у ветки EDT в имя входит ещё и набор исходников —
`syntax_edt_<набор>_*.log`.

## Пример

```json
{
  "provider": {"selected": "designer", "origin": {"kind": "default"}},
  "provider_dispatched": true,
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

Превью той же команды:

```json
{
  "provider": {"selected": "designer", "origin": {"kind": "default"}},
  "provider_dispatched": false,
  "status": "planned",
  "exit_code": -1,
  "check_name": "designer-config",
  "issues": [],
  "summary": {
    "errors": 0,
    "warnings": 0,
    "info": 0
  },
  "duration_ms": 3,
  "message": "would run `/CheckConfig -ThinClient -Server` via /opt/1cv8/x86_64/8.3.27.1000/1cv8; configuration not checked"
}
```
