# Судьба записей прежнего слоя

Прежний нормативный слой — тридцать один ADR и общий файл инвариантов — заморожен в
[`adr-v1/`](adr-v1/) и защищён [манифестом](adr-v1-manifest.md). Эта таблица — единственный
маршрут из него в действующий [`spec/arch/`](../arch/README.md). У каждой записи ровно одна судьба:

- `carried` — обязательство сохранено без смены смысла, у него один владелец;
- `superseded` — предмет разошёлся на несколько атомарных записей или пересмотрен;
- `retired` — отдельного обязательства больше нет; код и тесты остаются более высоким
  источником истины и поведение могут сохранять.

| Запись | Судьба | Владелец в `spec/arch` | Причина |
| --- | --- | --- | --- |
| [`0001-granitsy-podderzhki-ibcmd-kak-ogranichennogo-backend.md`](adr-v1/0001-granitsy-podderzhki-ibcmd-kak-ogranichennogo-backend.md) | superseded | `DEC.2026-09-14.PROVIDER-CHOSEN-PER-OPERATION`, `DEC.2026-09-14.PROVIDER-DEFAULTS-LIVE-IN-CODE` | — |
| [`0002-izolirovat-runtime-state-po-source-set-pod-workpath.md`](adr-v1/0002-izolirovat-runtime-state-po-source-set-pod-workpath.md) | superseded | `DEC.2026-04-20.SOURCE-SET-IS-THE-UNIT-OF-ORCHESTRATION`, `DEC.2026-04-20.WORKPATH-IS-THE-ONLY-STATE-ROOT`, `DEC.2026-04-20.EDT-EXPORT-KEEPS-ITS-OWN-CHANGE-STATE` | — |
| [`0003-podderzhivat-servernye-ib-dlya-vseh-instrumentov.md`](adr-v1/0003-podderzhivat-servernye-ib-dlya-vseh-instrumentov.md) | carried | `DEC.2026-04-20.SERVER-INFOBASES-ARE-A-TARGET-CONTRACT` | — |
| [`0004-avtoobnaruzhivat-komponenty-platformy-1s-po-versii-maske.md`](adr-v1/0004-avtoobnaruzhivat-komponenty-platformy-1s-po-versii-maske.md) | carried | `DEC.2026-04-20.PLATFORM-TOOLS-ARE-FOUND-BY-VERSION-MASK` | — |
| [`0005-razdelit-cli-i-mcp-publichnye-poverhnosti.md`](adr-v1/0005-razdelit-cli-i-mcp-publichnye-poverhnosti.md) | superseded | `DEC.2026-04-20.MCP-DOES-NOT-MIRROR-CLI`, `CTR.MCP.PUBLISHED-TOOL-SURFACE`, `INV.MCP.SURFACE-STAYS-EXPLICIT` | — |
| [`0006-sohranyat-transportno-neytralnyy-use-case-sloy.md`](adr-v1/0006-sohranyat-transportno-neytralnyy-use-case-sloy.md) | carried | `DEC.2026-04-20.USE-CASES-STAY-TRANSPORT-NEUTRAL` | — |
| [`0007-vydelit-otdelnyy-pereklyuchatel-dlya-shared-edt.md`](adr-v1/0007-vydelit-otdelnyy-pereklyuchatel-dlya-shared-edt.md) | superseded | `DEC.2026-04-20.EDT-RUNS-ONE-SHOT-OR-IN-ONE-SHARED-SESSION`, `DEC.2026-04-20.ONLY-A-LONG-LIVED-HOST-PREWARMS-EDT` | — |
| [`0008-derzhat-platformennye-backend-dsl-otdelno-ot-orchestration.md`](adr-v1/0008-derzhat-platformennye-backend-dsl-otdelno-ot-orchestration.md) | carried | `DEC.2026-04-20.PLATFORM-DSL-STAYS-OUT-OF-ORCHESTRATION` | — |
| [`0009-razdelit-business-i-transport-runtime-failures.md`](adr-v1/0009-razdelit-business-i-transport-runtime-failures.md) | carried | `DEC.2026-04-20.BUSINESS-FAILURES-ARE-NOT-TRANSPORT-FAULTS` | — |
| [`0010-razdelit-cli-output-dlya-cheloveka-i-ai-agenta.md`](adr-v1/0010-razdelit-cli-output-dlya-cheloveka-i-ai-agenta.md) | superseded | `DEC.2026-04-20.ONE-OUTPUT-CONTRACT-FOR-HUMAN-AND-AGENT`, `DEC.2026-04-20.JSON-MESSAGE-IS-THE-ONLY-FORMAT-SWITCH` | — |
| [`0011-eksklyuzivnoe-vladenie-workpath-na-vremya-komandy.md`](adr-v1/0011-eksklyuzivnoe-vladenie-workpath-na-vremya-komandy.md) | superseded | `DEC.2026-04-20.A-COMMAND-OWNS-THE-WORKPATH-EXCLUSIVELY`, `DEC.2026-04-20.THE-OS-LOCK-IS-THE-TRUTH-THE-SIDECAR-IS-DIAGNOSTICS` | — |
| [`0012-on-demand-change-detection-i-faylovaya-partial-load-strategiya.md`](adr-v1/0012-on-demand-change-detection-i-faylovaya-partial-load-strategiya.md) | superseded | `DEC.2026-04-20.CHANGES-ARE-DETECTED-ON-DEMAND`, `DEC.2026-04-20.DOUBT-TURNS-A-PARTIAL-LOAD-INTO-A-FULL-ONE` | — |
| [`0013-mcp-execution-admission-timeout-cancellation-routing-i-http-session-capacity.md`](adr-v1/0013-mcp-execution-admission-timeout-cancellation-routing-i-http-session-capacity.md) | carried | `DEC.2026-04-20.MCP-LIMITS-EXECUTION-AND-SESSIONS-SEPARATELY` | — |
| [`0014-edinaya-timeout-cancellation-policy-dlya-cli-i-mcp-komand.md`](adr-v1/0014-edinaya-timeout-cancellation-policy-dlya-cli-i-mcp-komand.md) | superseded | `DEC.2026-04-20.EVERY-COMMAND-HAS-A-DEADLINE`, `DEC.2026-04-20.CANCELLATION-COUNTS-ONLY-AFTER-A-TERMINAL-STATE`, `DEC.2026-04-20.A-MUTATING-CRITICAL-PHASE-IS-NOT-HARD-KILLED` | — |
| [`0015-atomarnaya-publikatsiya-dump-artifacts-cherez-staging-backup.md`](adr-v1/0015-atomarnaya-publikatsiya-dump-artifacts-cherez-staging-backup.md) | carried | `DEC.2026-04-21.FULL-REPLACEMENT-PUBLISHES-THROUGH-STAGING` | — |
| [`0016-edinyy-executionoutcome-i-pipeline-steps-dlya-runner-like-stsenariev.md`](adr-v1/0016-edinyy-executionoutcome-i-pipeline-steps-dlya-runner-like-stsenariev.md) | superseded | `DEC.2026-04-21.A-COMMAND-IS-A-PIPELINE-OF-TYPED-BLOCKS`, `DEC.2026-04-21.EXECUTION-OUTCOME-IS-THE-CANONICAL-RESULT` | — |
| [`0017-v8project-yaml-source-set-kak-glavnyy-konfiguratsionnyy-kontrakt.md`](adr-v1/0017-v8project-yaml-source-set-kak-glavnyy-konfiguratsionnyy-kontrakt.md) | superseded | `DEC.2026-04-20.V8PROJECT-YAML-IS-THE-PROJECT-CONTRACT`, `DEC.2026-04-20.SOURCE-SET-IS-THE-UNIT-OF-ORCHESTRATION`, `DEC.2026-04-20.SOURCE-SET-TYPE-IS-DETECTED-FROM-MARKER-CONTENT` | — |
| [`0018-perenesti-kontrakt-informatsionnoy-bazy-v-infobase.md`](adr-v1/0018-perenesti-kontrakt-informatsionnoy-bazy-v-infobase.md) | carried | `DEC.2026-04-21.INFOBASE-SECTION-OWNS-CONNECTION-AND-CREDENTIALS` | — |
| [`0019-sozdavat-servernuyu-infobazu-cherez-ibcmd-pri-init-pri-otsutstvii.md`](adr-v1/0019-sozdavat-servernuyu-infobazu-cherez-ibcmd-pri-init-pri-otsutstvii.md) | carried | `DEC.2026-04-22.INIT-ENSURES-A-SERVER-INFOBASE-THROUGH-IBCMD` | — |
| [`0020-dobavit-cli-only-convert-dlya-dvustoronney-konvertatsii-edt-i-designer.md`](adr-v1/0020-dobavit-cli-only-convert-dlya-dvustoronney-konvertatsii-edt-i-designer.md) | carried | `DEC.2026-04-22.CONVERT-WORKS-ON-PROJECT-SOURCE-SETS` | — |
| [`0021-lokalnyy-overlay-config.md`](adr-v1/0021-lokalnyy-overlay-config.md) | superseded | `DEC.2026-05-02.LOCAL-OVERLAY-CARRIES-MACHINE-SETTINGS`, `DEC.2026-05-02.THE-OVERLAY-CANNOT-CHANGE-PROJECT-IDENTITY` | — |
| [`0022-universalnyy-mehanizm-podgotovki-rasshireniy-i-client-mcp-extension.md`](adr-v1/0022-universalnyy-mehanizm-podgotovki-rasshireniy-i-client-mcp-extension.md) | carried | `DEC.2026-05-02.TOOL-EXTENSIONS-ARE-NOT-PROJECT-SOURCE-SETS` | — |
| [`0023-fail-closed-sostoyaniya-proverki-zagruzki.md`](adr-v1/0023-fail-closed-sostoyaniya-proverki-zagruzki.md) | superseded | `DEC.2026-09-02.LOAD-COMPATIBILITY-STATES-ARE-CLOSED-AND-FAIL-CLOSED` | — |
| [`0024-tipizirovat-eksport-konfiguratsii-i-snimka-ib.md`](adr-v1/0024-tipizirovat-eksport-konfiguratsii-i-snimka-ib.md) | superseded | `DEC.2026-09-02.EXPORT-INTENTS-ARE-TYPED-SEPARATELY`, `DEC.2026-09-11.PREVIEW-STOPS-BEFORE-THE-PROVIDER-IS-DISPATCHED` | — |
| [`0025-nevypolnyayuschee-prevyu-zapuska-klienta.md`](adr-v1/0025-nevypolnyayuschee-prevyu-zapuska-klienta.md) | superseded | `DEC.2026-09-11.PREVIEW-STOPS-BEFORE-THE-PROVIDER-IS-DISPATCHED`, `DEC.2026-09-11.SECRETS-ARE-MASKED-IN-EVERY-OUTPUT` | — |
| [`0026-zagruzka-informatsionnoy-bazy-iz-dt.md`](adr-v1/0026-zagruzka-informatsionnoy-bazy-iz-dt.md) | carried | `DEC.2026-09-11.RESTORE-REQUIRES-AN-EXPLICIT-TARGET-MODE` | — |
| [`0027-sostav-rasshireniy-informatsionnoy-bazy.md`](adr-v1/0027-sostav-rasshireniy-informatsionnoy-bazy.md) | superseded | `DEC.2026-09-11.BASE-EXTENSIONS-ARE-A-SEPARATE-SUBJECT`, `DEC.2026-09-11.LISTING-OUTPUT-IS-PARSED-FAIL-CLOSED` | — |
| [`0028-prevyu-u-glagolov-bez-vybora-provaydera.md`](adr-v1/0028-prevyu-u-glagolov-bez-vybora-provaydera.md) | superseded | `DEC.2026-09-11.PREVIEW-NAMES-THE-SUBJECT-NOT-A-FAKE-FORK`, `DEC.2026-09-11.PREVIEW-DOES-NOT-TAKE-THE-LOCK` | — |
| [`0029-proza-instrumenta-ne-prinimaet-resheniy.md`](adr-v1/0029-proza-instrumenta-ne-prinimaet-resheniy.md) | superseded | `DEC.2026-09-12.TOOL-PROSE-NEVER-DECIDES`, `DEC.2026-09-12.A-LABEL-MAY-ONLY-MAKE-A-VERDICT-STRICTER` | — |
| [`0030-provaydery-po-operatsiyam-s-umolchaniyami-v-kode.md`](adr-v1/0030-provaydery-po-operatsiyam-s-umolchaniyami-v-kode.md) | superseded | `DEC.2026-09-14.PROVIDER-CHOSEN-PER-OPERATION`, `DEC.2026-09-14.PROVIDER-DEFAULTS-LIVE-IN-CODE`, `DEC.2026-09-14.PROVIDER-OVERRIDE-IS-STRICT`, `DEC.2026-09-14.PROVIDER-IS-NOT-A-CALL-ARGUMENT`, `DEC.2026-09-14.BUILDER-KEY-IS-REMOVED`, `DEC.2026-09-14.RECEIPT-EXPLAINS-PROVIDER-CHOICE`, `DEC.2026-09-14.AGENT-SESSION-LIVES-WITH-THE-LOCK`, `DEC.2026-09-14.AGENT-SPEAKS-JSON-WITHOUT-A-PTY`, `DEC.2026-09-14.AGENT-ENDPOINT-IS-MANAGED-OR-ATTACHED`, `DEC.2026-09-14.DBMS-SECTION-IS-DATABASE-ACCESS` | — |
| [`invariants.md`](adr-v1/invariants.md) | superseded | реестр `spec/arch/invariants` и `spec/arch/contracts` | — |

## Правила без проверки

Утверждения прежнего файла, у которых не нашлось названного падающего теста, стали
правилами со `status: planned` и `check: null`. Индекс показывает это колонкой
«проверяется», поэтому долг виден и считается: сейчас таких правил двадцать — девять ждут теста, одиннадцать ждут кода.
Правило переводится в действующее тем же изменением, которое приносит тест.
