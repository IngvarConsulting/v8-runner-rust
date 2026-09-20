## 9. Архитектурные решения

Источник истины для архитектурных решений — атомарный реестр [`spec/arch/`](../../arch/index.md): решения `DEC.*`, инварианты `INV.*`, контракты `CTR.*`. Прежний нумерованный слой заморожен в [`spec/archive/adr-v1/`](../../archive/adr-v1/), и у каждой его записи ровно один владелец в реестре по [таблице судьбы](../../archive/FATE.md). Этот раздел не дублирует индекс, а называет контракты, которые arc42 обязан отражать в остальных разделах.

Архитектурные правила для агентов и контрибьюторов живут в реестре [spec/arch](../../arch/README.md).

### Сквозные выводы из ADR

- Public surface changes нужно оценивать отдельно для CLI и MCP: наличие CLI-команды не означает доступность MCP tool.
- `convert` является осознанной CLI-only командой и не должен трактоваться как автоматический кандидат в MCP tool.
- Use case layer остаётся общей транспортно-нейтральной orchestration boundary, а adapters отвечают за presentation, DTO и transport/runtime failures.
- `source-set.name` и canonical `workPath` являются runtime identity. Изменения naming/path rules затрагивают config validation, change detection, generated directories и workspace lock.
- `infobase` является единственным config contract для строки подключения, пользователя ИБ и DBMS-level доступа; top-level `connection`/`credentials` не поддерживаются.
- Полный `infobase.dbms` contract при `builder=IBCMD` достаточно явно разрешает server infobase provisioning в `init`; отдельный top-level provisioning flag для этого не требуется.
- Repo-aware `convert` и reverse sync из ИБ в файлы — разные сценарии; `dump format=EDT` реализован как отдельный flow поверх internal Designer snapshot и EDT import, а не как alias или скрытый sub-step `convert`.
- MCP concurrency имеет два независимых контура: execution admission для tool calls и HTTP session capacity для stateful transport lifecycle.
- Target publication safety не обеспечивается workspace lock: full replacement outputs требуют staging/backup contract рядом с target.
- `DEC.2026-09-20.A-COMMAND-HAS-NO-DEADLINE` и `DEC.2026-04-21.A-COMMAND-IS-A-PIPELINE-OF-TYPED-BLOCKS` описывают целевую архитектуру с известными migration gaps. Новые команды должны следовать этим контрактам, даже если часть старых сценариев ещё находится в переходном состоянии.

### Правила актуализации

- При добавлении или изменении ADR синхронизировать этот раздел и затронутые arc42-разделы, а не только список ссылок.
- При изменении любого инварианта сначала обновлять соответствующий ADR или добавлять новый ADR, который явно заменяет старое решение.
- Если реализация временно расходится с принятым ADR, фиксировать это как implementation gap в разделе 11 и в профильной публичной документации, когда gap виден пользователю.
