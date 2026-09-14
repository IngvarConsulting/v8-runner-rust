# ADR-0019: Обеспечивать наличие серверной ИБ через `ibcmd` в `init`

- Статус: `accepted`
- Дата: `2026-04-22`
- Связанные решения: [ADR-0003](0003-podderzhivat-servernye-ib-dlya-vseh-instrumentov.md), [ADR-0018](0018-perenesti-kontrakt-informatsionnoy-bazy-v-infobase.md)

## Контекст

`ADR-0003` зафиксировал server infobase support как обязательный целевой контракт всех инструментов, но оставил backend-specific детали последующим решениям.
`ADR-0018` уже перенёс DBMS-level contract в `infobase.dbms`, то есть в конфиге появился явный набор параметров для server-based ИБ:

- `kind`
- `server`
- `name`
- optional `user`
- optional `password`

Этого достаточно, чтобы `IBCMD` не только работать с уже существующей server-based ИБ, но и обеспечивать её наличие через `ibcmd infobase create --create-database`.
При этом в локально зафиксированном `ibcmd` spec нет отдельной явной `infobase info/list` команды для existence check, поэтому архитектурный контракт не должен зависеть от обязательного pre-check перед create.

Текущее правило "server connection в `init` всегда означает manual prerequisite и skipped create step" создаёт несколько проблем:

1. `init` ведёт себя асимметрично для file и server connections.
2. Автоматизация и AI-агенты вынуждены делать внешний provisioning step вне `v8-runner`.
3. `builder=IBCMD` остаётся уже, чем допускает уже принятый config contract из `ADR-0018`.
4. В документации начинают смешиваться два разных утверждения:
   - `infobase.dbms` уже является явным server contract;
   - server infobase всё ещё должна создаваться только вручную.

Нужно зафиксировать, как именно `init` должен использовать уже принятый `infobase.dbms` contract для server-based ИБ.

## Решение

Для `init` принять server infobase provisioning через `IBCMD` как поддержанный ensure-сценарий при наличии полного DBMS-level contract.

Правила:

1. Для `builder=IBCMD` + server-based `infobase.connection` команда `init` должна обеспечивать наличие ИБ, а не безусловно пропускать infobase step.
2. Existing `infobase.dbms` contract из `ADR-0018` считается достаточным explicit authorization для server provisioning path; отдельный top-level флаг для этого не вводится.
3. Для этого сценария обязательны `infobase.dbms.kind`, `infobase.dbms.server` и `infobase.dbms.name`; optional `infobase.dbms.user/password` передаются как DBMS credentials.
4. `ibcmd infobase create --create-database` используется как ensure-step; отдельный обязательный read-only pre-check существования ИБ не требуется.
5. Наличие ИБ определяется по нормализованному результату ensure-step:
   - success означает, что ИБ готова к дальнейшему сценарию;
   - известный benign сигнал вида "already exists" трактуется как non-error outcome;
   - auth, network, permission и другие ambiguous failures не трактуются как "ИБ отсутствует".
6. Нормализация benign outcomes должна жить в platform adapter boundary, а не в use-case слое.
7. `infobase.user/password` остаются учетными данными пользователя информационной базы и также передаются в `IBCMD`; DBMS-level credentials их не заменяют.
8. Если целевая server-based ИБ уже существует, итог должен быть non-error: `Skipped` или `Ok` с явным сообщением, что база уже готова.
9. Поведение для file-based ИБ не меняется: локальная файловая база по-прежнему создаётся существующим file path flow.
10. Этот ADR уточняет и заменяет старую формулировку "server init всегда manual prerequisite" из `ADR-0003` и `ADR-0018` только для `builder=IBCMD` provisioning path.
11. Для `builder=DESIGNER` автоматическое создание server-based ИБ этим ADR не вводится.

## Неграницы (Non-goals)

1. Автоматическое создание server cluster, пользователей, ролей или прав доступа.
2. Вывод DBMS-level параметров из строки `Srvr=...;Ref=...`.
3. Добавление отдельного `tools.*` или другого top-level config field для server provisioning path.
4. Немедленное распространение этого решения на `builder=DESIGNER`.
5. Destructive recreate существующей server-based ИБ.

## Последствия

1. `init` становится пригодным для более полного automation flow при `builder=IBCMD`.
2. `infobase.dbms` перестаёт быть только "contract для уже существующей server-based ИБ" и становится также contract для её provisioning path.
3. Старые формулировки про mandatory manual prerequisite для server init должны быть убраны или явно заменены ссылкой на этот ADR.
4. Validation и platform mapping должны различать:
   - неполный DBMS contract как user-facing validation error;
   - "already exists" как non-fatal init outcome;
   - реальный provisioning failure как failed step.
5. До реализации этого ADR текущий skip-path в `init` считается implementation gap.

## План реализации

1. Довести config model из `ADR-0018` до рабочего состояния:
   - typed `infobase` / `infobase.dbms`;
   - отказ от legacy top-level `connection` / `credentials`;
   - validation boundary для server/file combinations.
2. Расширить `src/platform/ibcmd.rs`:
   - добавить file/server variants в `IbcmdConnection`;
   - поддержать `infobase create` для server-based ИБ;
   - добавить `--create-database` для server provisioning path;
   - нормализовать benign signal вида "already exists" в typed non-fatal outcome;
   - сохранить отдельную передачу infobase credentials и DBMS credentials.
3. Обновить `src/use_cases/init_project.rs`:
   - убрать unconditional skip для server connection при `builder=IBCMD`;
   - вызывать server provisioning path через `IbcmdDsl` как ensure-step;
   - трактовать benign "already exists" как non-error init step;
   - не трактовать auth/network/permission errors как "базы нет";
   - сохранить EDT workspace step независимым от infobase step.
4. Обновить документацию:
   - `spec/decisions/README.md`;
   - `spec/architecture/invariants.md`;
   - arc42 decisions / risks;
   - пользовательские docs после фактической реализации behavior.
5. Обновить tests:
   - config validation tests для `infobase.dbms`;
   - `IbcmdConnection` / args mapping tests для server create path;
   - `init` tests на server provisioning, existing database и failure mapping.

## Верификация

- [x] ADR фиксирует, что `builder=IBCMD` может provisioning-ить missing server-based ИБ в `init`.
- [x] ADR фиксирует, что explicit config contract для этого path уже задаётся через `infobase.dbms`.
- [x] ADR фиксирует, что server infobase provisioning выполняется как ensure-step без обязательного отдельного pre-check.
- [x] ADR фиксирует, что benign "already exists" нормализуется как non-error outcome, а не как failed create.
- [x] ADR сохраняет разделение infobase credentials и DBMS credentials.
- [x] ADR сохраняет file-based `init` behavior без изменения.
- [x] ADR явно ограничивает решение `IBCMD`-веткой и не распространяет его автоматически на `DESIGNER`.
- [x] ADR помечает текущий unconditional skip server create в `init` как implementation gap до реализации.
