## 9. Архитектурные решения

Согласованные гарантии продукта живут правилами в [`spec/arch/rules/`](../../arch/README.md):
одно правило — один файл, и каждое называет проверку, которая падает при нарушении.
Отдельного реестра решений нет: почему правило таково, рассказывают задача и PR, которыми
оно заведено, а прежние формы лежат в истории Git.

Этот раздел не пересказывает правила. Он называет сквозные следствия, которые arc42 обязан
отражать в остальных разделах.

### Сквозные следствия

- Public surface changes нужно оценивать отдельно для CLI и MCP: наличие CLI-команды не означает доступность MCP tool.
- `convert` является осознанной CLI-only командой и не должен трактоваться как автоматический кандидат в MCP tool.
- Use case layer остаётся общей транспортно-нейтральной orchestration boundary, а adapters отвечают за presentation, DTO и transport/runtime failures.
- `source-set.name` и canonical `workPath` являются runtime identity. Изменения naming/path rules затрагивают config validation, change detection, generated directories и workspace lock.
- `infobase` является единственным config contract для строки подключения, пользователя ИБ и DBMS-level доступа; top-level `connection`/`credentials` не поддерживаются.
- Полный `infobase.dbms` contract при `providers.infobase.create: ibcmd` достаточно явно разрешает server infobase provisioning в `infobase create`; отдельный top-level provisioning flag для этого не требуется.
- Repo-aware `convert` и reverse sync из ИБ в файлы — разные сценарии; `pull format=EDT` реализован как отдельный flow поверх internal Designer snapshot и EDT import, а не как alias или скрытый sub-step `convert`.
- MCP concurrency имеет два независимых контура: execution admission для tool calls и HTTP session capacity для stateful transport lifecycle.
- Target publication safety не обеспечивается workspace lock: full replacement outputs требуют staging/backup contract рядом с target.
- У публичной команды нет предельного времени выполнения: предел бывает только у шага и только если шаг объявил его сам. Сложный сценарий собирается из типизированных блоков, обменивающихся типизированным контекстом. Обе гарантии описывают целевую архитектуру с известными migration gaps; новые команды следуют им, даже если часть старых сценариев ещё в переходном состоянии.

### Правила актуализации

- При изменении гарантии сначала правят её правило в `spec/arch/rules/`, затем — затронутые разделы arc42.
- Если реализация временно расходится с согласованной гарантией, у правила стоит `gap` со ссылкой на задачу, а сам разрыв описан в разделе 11 и в профильной публичной документации, когда он виден пользователю.
