---
id: DEC.2026-09-21.CHECK-IS-CHECKCONFIG
status: active
governs: product
realized: tests/cli_syntax.rs::check_takes_its_modes_without_a_subcommand
supersedes: []
superseded-by: null
establishes: []
changes: [CTR.WIRE.SYNTAX-DATA]
---

# Одна команда `check`

**Решение.** Проверка конфигурации — одна команда `check`: для формата платформы это
`/CheckConfig` со всеми режимами — целостность, ссылки, синтаксис по режимам,
обработчики, модальность, расширенная проверка модулей, — для формата EDT это проверка
проекта средствами EDT CLI, и база тогда не нужна. Исполнитель один, ключа
`providers.check` нет. У автономного сервера проверка идёт по прямому шлюзу; в наборе
SSH-шлюза проверок нет. Для внешних обработок и отчётов проверка не описана — отказ рода
подбора `subject`. Сверх сайта: инструменты MCP `check_syntax_edt`,
`check_syntax_designer_config` и `check_syntax_designer_modules` сохраняют имена и схемы
входа, исполняются тем же сценарием `check` — режимы проверки модулей берутся у
`/CheckConfig` — и отвечают его формой; отдельного пути `/CheckModules` на поверхности
не остаётся.

**Почему.** Три команды `syntax designer-config`, `syntax designer-modules` и
`syntax edt` делили одно намерение по инструменту, и вызывающему приходилось знать, чем
платформа проверяет модули, чтобы попросить проверку.

**Не затрагивает.** `CTR.MCP.PUBLISHED-TOOL-SURFACE` и состав режимов `/CheckConfig`: он
остаётся платформенным и перечисляется в справке команды.

Источник: [`cli.html#map`](../../../docs/site/cli.html#map),
[`scenarios.html`](../../../docs/site/scenarios.html).
