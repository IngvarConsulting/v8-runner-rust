// Данные конфигуратора сценариев. Ключи базы записаны от её секции: connection значит infobases.<имя>.connection. Факты — из docs/CAPABILITIES.md, docs/CONFIGURATION.md,
// реестра spec/arch и замеров 12–14.09.2026. Всё, чего здесь нет, конфигуратор не обещает.
//
// Сайт описывает целевую модель.
window.RUNNER_DATA = (function () {
  var P = {
    designer: { key: 'designer', name: 'Designer', cls: 'batch', needs: 'designer' },
    // Агентскую точку входа для файловой и кластерной базы создаёт сам раннер: это
    // процесс Конфигуратора, поднятый в агентском режиме, — значит на машине раннера
    // нужна платформа. Подключиться к уже работающей точке входа можно только у
    // автономного сервера: он держит свой SSH-шлюз независимо от раннера.
    agent:    { key: 'agent',    name: 'agent',    cls: 'agent',
                needs: function (ctx) {
                  return ctx.tools.agent && (ctx.target === 'standalone' || ctx.tools.designer);
                } },
    ibcmd:    { key: 'ibcmd',    name: 'ibcmd',    cls: 'ibcmd', needs: 'ibcmd' },
    rs:       { key: 'ibcmd-rs', name: 'ibcmd-rs', cls: 'rs',    needs: 'rs' },
    edt:      { key: 'edt-cli',  name: 'EDT CLI',  cls: 'neutral', needs: 'edt' },
    client:   { key: 'client',   name: 'клиент 1С', cls: 'neutral', needs: 'designer' },
    webinst:  { key: 'webinst',  name: 'webinst',   cls: 'neutral', needs: 'web' },
    rac:      { key: 'rac',      name: 'rac',       cls: 'neutral', needs: 'rac' },
    browser:  { key: 'browser',  name: 'браузер',   cls: 'neutral', needs: 'browser' }
  };

  // Что означает каждая ось выбора
  var AXES = {
    format: [
      { id: 'DESIGNER', label: 'XML платформы', hint: 'каталог с Configuration.xml — то, что выгружает Конфигуратор' },
      { id: 'EDT',      label: 'проект EDT',   hint: 'каталог с .project и DT-INF/PROJECT.PMF' }
    ],
    type: [
      { id: 'CONFIGURATION', label: 'конфигурация', hint: 'основная конфигурация базы' },
      { id: 'EXTENSION',     label: 'расширение', hint: 'в проекте нужна хотя бы одна конфигурация' },
      { id: 'EXTERNAL',      label: 'внешние', hint: 'обработки и отчёты: в базу не загружаются, собираются в .epf и .erf' }
    ],
    target: [
      { id: 'file',       label: 'файловая',  hint: 'File=…; раннер может создать её сам. Агента для неё поднимает раннер — нужна платформа' },
      { id: 'cluster',    label: 'кластер 1С', hint: 'Srvr=…;Ref=…; данные СУБД нужны только чтобы создать базу. Агента для неё поднимает раннер — нужна платформа' },
      { id: 'standalone', label: 'автономный сервер', hint: 'секция standalone; Конфигуратор идёт в прямой шлюз как в кластер, SSH-шлюз сервера даёт агентский набор без платформы у раннера' }
    ],
    tools: [
      { id: 'designer', label: 'платформа 1С (1cv8, Конфигуратор)', short: 'платформа 1С', def: true },
      { id: 'ibcmd',    label: 'ibcmd', short: 'ibcmd', def: true },
      { id: 'edt',      label: 'EDT и 1cedtcli', short: 'EDT', def: false },
      { id: 'agent',    label: 'агентский режим', short: 'агент', def: true },
      { id: 'rs',       label: 'ibcmd-rs', short: 'ibcmd-rs', def: false },
      { id: 'web',      label: 'веб-сервер Apache или IIS', short: 'веб-сервер', def: false },
      { id: 'rac',      label: 'rac и сервер администрирования кластера', short: 'rac', def: false }
    ]
  };

  // Сценарии. Для каждого: applies(ctx) → null (применим) или причина;
  // target(ctx) → {chain:[P…], config:[…], note} по целевой матрице исполнителей.
  // Причины отказа: subject — неподходящий предмет, target — неподходящая цель,
  // tool — отсутствует необходимый инструмент.
  function needEdt(ctx) {
    return ctx.format === 'EDT' && !ctx.tools.edt
      ? { kind: 'tool', why: 'нет EDT CLI', fix: 'поставьте EDT, укажите tools.edt_cli.path' }
      : null;
  }
  function notExternal(ctx, verb) {
    return ctx.type === 'EXTERNAL'
      ? { kind: 'subject', why: 'внешние обработки в базу не загружаются', fix: 'для них — make и convert' }
      : null;
  }
  // У автономного сервера два канала: Конфигуратор по прямому шлюзу, как в кластер,
  // и SSH-шлюз с урезанным агентским набором. Отказывает у него только создание базы:
  // её создают ibcmd до запуска сервера, на его машине.
  function standaloneRefuses(ctx, why) {
    return ctx.target === 'standalone' ? { kind: 'target', why: why, fix: '' } : null;
  }
  // Инструмент провайдера бывает один (`needs: 'ibcmd'`), а бывает условием от цели:
  // тогда `needs` — функция от контекста.
  function ready(provider, ctx) {
    return typeof provider.needs === 'function'
      ? !!provider.needs(ctx)
      : !!ctx.tools[provider.needs];
  }

  // Почему провайдер не готов. Отмеченный инструмент ещё не делает исполнителя
  // доступным: агенту для файловой и кластерной базы нужна платформа, потому что
  // поднимает его раннер.
  function missingFor(provider, ctx) {
    if (provider.key === 'agent' && ctx.tools.agent && ctx.target !== 'standalone' && !ctx.tools.designer) {
      return 'agent (его поднимает раннер — нужна платформа)';
    }
    return provider.name;
  }

  function firstReady(chain, ctx) {
    for (var i = 0; i < chain.length; i++) { if (ready(chain[i], ctx)) return chain[i]; }
    return null;
  }

  var SCENARIOS = [
    {
      id: 'status', verb: 'status', title: 'Понять, что происходит',
      what: 'Называет, к какой базе привязан каталог, что разошлось и что потеряется при замене.',
      cmd: function (ctx) { return 'v8-runner status'; },
      applies: function () { return null; },
      target: function (ctx) { return { chain: [], config: [], note: 'платформу не запускает и базу не трогает: это первая команда, когда непонятно' }; }
    },
    {
      id: 'init', verb: 'init', title: 'Завести проект здесь',
      what: 'Пишет v8project.yaml по найденным исходникам.',
      cmd: function (ctx) { return 'v8-runner init'; },
      applies: function () { return null; },
      target: function (ctx) { return { chain: [], config: [], note: 'платформа не нужна; тип каждого набора определяется по содержимому файлов, не по именам каталогов' }; }
    },
    {
      id: 'clone', verb: 'clone', title: 'Завести проект из существующей базы',
      what: 'Привязывает каталог к базе и делает первую выгрузку.',
      cmd: function (ctx) { return 'v8-runner clone --from "File=/srv/ib/demo"'; },
      applies: function (ctx) { return notExternal(ctx, 'clone'); },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { chain: [P.designer, P.agent], config: ['connection или standalone.gate'], note: 'пишет v8project.yaml, местный слой и делает pull' };
        if (ctx.target === 'cluster') return { chain: [P.agent, P.designer], config: [], note: 'пишет v8project.yaml, местный слой и делает pull' };
        return { chain: [P.agent, P.designer, P.ibcmd], config: [], note: 'пишет v8project.yaml, местный слой и делает pull — цепочка та же, что у pull' };
      }
    },
    {
      id: 'infobase-create', verb: 'infobase create', title: 'Создать базу',
      what: 'Создаёт базу, если её нет.',
      cmd: function (ctx) { return 'v8-runner infobase create'; },
      applies: function (ctx) { return standaloneRefuses(ctx, 'базу автономного сервера создают ibcmd до запуска сервера, на его машине; раннер к нему подключается, ничего не запуская') || needEdt(ctx); },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { kind: 'target', why: 'базу автономного сервера создают до его запуска, на его машине', fix: 'ibcmd server config init, затем ibcmd infobase create --load|--import|--restore; раннер подключается к уже работающему шлюзу' };
        if (ctx.target === 'cluster') return { chain: [P.designer, P.rac], config: ['connection', 'dbms.*', 'cluster.user — если в кластере заведены администраторы'], note: 'CREATEINFOBASE с клиент-серверной строкой: регистрация в кластере и CrSQLDB=Y одной командой; rac — запасной путь' };
        return { chain: [P.ibcmd, P.designer], config: ['connection'], note: 'ibcmd создаёт файловую базу сразу с конфигурацией из исходников (--import); память о базе записывается сразу, первый push идёт как обычный' };
      }
    },
    {
      id: 'push', verb: 'push', title: 'Отправить исходники в базу',
      what: 'Отправляет изменённые исходники в основную конфигурацию и применяет к базе данных; --no-apply останавливается раньше.',
      cmd: function (ctx) { return 'v8-runner push'; },
      applies: function (ctx) { return notExternal(ctx, 'push') || needEdt(ctx); },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { chain: [P.designer, P.agent], config: ['connection — прямой шлюз', 'standalone.gate и exchange — для agent', 'source-set[]'], note: 'Конфигуратор по прямому шлюзу читает исходники у раннера; agent — load-config-from-files + update-db-cfg одной сессией, файлы через канал обмена' };
        if (ctx.target === 'cluster') return { chain: [P.agent, P.designer], config: ['connection', 'source-set[]'], note: 'одна сессия агента на всю команду; без агента — Designer' };
        return { chain: [P.agent, P.designer, P.ibcmd], config: ['connection', 'source-set[]'], note: 'одна сессия агента на всю команду; без агента — Designer' };
      }
    },
    {
      id: 'apply', verb: 'apply', title: 'Применить к базе данных',
      what: 'Приводит конфигурацию базы данных к основной; --sessions force разрешает завершать чужие сеансы.',
      cmd: function (ctx) { return 'v8-runner apply'; },
      applies: function (ctx) { return notExternal(ctx, 'apply') || needEdt(ctx); },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { chain: [P.designer, P.agent], config: ['connection', 'standalone.gate — для agent'], note: '/UpdateDBCfg по прямому шлюзу; agent — update-db-cfg по SSH' };
        if (ctx.target === 'cluster') return { chain: [P.agent, P.designer], config: ['connection'], note: '/UpdateDBCfg' };
        return { chain: [P.agent, P.designer, P.ibcmd], config: ['connection'], note: '/UpdateDBCfg или ibcmd config apply' };
      }
    },
    {
      id: 'reset', verb: 'reset', title: 'Отбросить непринятое в базе',
      what: 'Возвращает основную конфигурацию к конфигурации базы данных; чужое непринятое без --force не трогает.',
      cmd: function (ctx) { return 'v8-runner reset'; },
      applies: function (ctx) { return notExternal(ctx, 'reset'); },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { chain: [P.designer], config: ['connection'], note: '/RollbackCfg по прямому шлюзу; в наборе SSH-шлюза отката нет' };
        if (ctx.target === 'cluster') return { chain: [P.designer], config: ['connection'], note: '/RollbackCfg, для расширения с -Extension' };
        return { chain: [P.designer, P.ibcmd], config: ['connection'], note: '/RollbackCfg или ibcmd config reset; своё непринятое — по памяти о поколении, чужое — только с --force' };
      }
    },
    {
      id: 'diff', verb: 'diff', title: 'Что изменилось в базе',
      what: 'Перечисляет объекты, изменившиеся в базе с последней синхронизации; --against сравнивает с базой данных, поставщиком или пакетом.',
      cmd: function (ctx) { return 'v8-runner diff'; },
      applies: function (ctx) { return notExternal(ctx, 'diff') || needEdt(ctx); },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { chain: [P.designer, P.agent], config: ['connection'], note: '/CompareCfg и /DumpConfigToFiles -getChanges по прямому шлюзу; agent — только по файлу версий, отчёта сравнения у SSH-шлюза нет' };
        if (ctx.target === 'cluster') return { chain: [P.designer], config: ['connection'], note: '/DumpConfigToFiles -getChanges по файлу версий; отчёты сравнения — /CompareCfg' };
        return { chain: [P.ibcmd, P.designer], config: ['connection'], note: 'ibcmd export status по файлу версий; отчёты сравнения — /CompareCfg' };
      }
    },
    {
      id: 'sessions', verb: 'sessions', title: 'Сеансы: список, завершение, блокировка',
      what: 'Окно обслуживания: запретить новые сеансы, завершить старые, после применения разрешить.',
      cmd: function (ctx) { return 'v8-runner sessions terminate --all'; },
      applies: function (ctx) { return ctx.target === 'file' ? { kind: 'target', why: 'у файловой базы нет сервера, который ведёт сеансы', fix: 'веб-сеансы завершает apply --sessions force в момент применения' } : notExternal(ctx, 'sessions'); },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { chain: [P.ibcmd], config: ['standalone.*'], note: 'ibcmd session list и terminate; блокировки начала сеансов у автономного сервера нет' };
        return { chain: [P.rac], config: ['cluster.ras — или раннер поднимет ras сам', 'cluster.user и password — администратор кластера', 'user и password — для deny и allow'], note: 'rac session и rac infobase update --sessions-deny; ras объявленный или поднятый раннером на время команды; исполнитель один, ключа нет' };
      }
    },
    {
      id: 'test', verb: 'test', title: 'Прогнать тесты',
      what: 'Запускает YAxUnit или Vanessa.',
      cmd: function (ctx) { return 'v8-runner test yaxunit all'; },
      applies: function (ctx) { return notExternal(ctx, 'test') || needEdt(ctx); },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { chain: [P.client], config: ['connection', 'web.url — для --via web', 'tests.yaxunit.* или tests.va.*'], note: 'тонкий клиент по прямому шлюзу или по HTTP с --via web; тесты в толстом клиенте недоступны — прямой шлюз его не пускает; сначала push, --no-push пропускает' };
        return { chain: [P.client], config: ['connection', 'tests.yaxunit.* или tests.va.*', 'tools.va.epf_path — для Vanessa'], note: 'сначала push, затем прогон; --no-push пропускает отправку' };
      }
    },
    {
      id: 'pull', verb: 'pull', title: 'Забрать изменения из базы',
      what: 'Выгружает конфигурацию базы в исходники.',
      cmd: function (ctx) { return 'v8-runner pull'; },
      applies: function (ctx) { return notExternal(ctx, 'pull') || needEdt(ctx); },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { chain: [P.designer, P.agent], config: ['connection', 'source-set[]', 'standalone.gate и exchange — для agent'], note: 'Конфигуратор пишет выгрузку у раннера; agent — dump-config-to-files по SSH, результат через объявленный канал обмена' };
        if (ctx.target === 'cluster') return { chain: [P.agent, P.designer], config: ['connection', 'source-set[]'], note: 'выгрузка агента соответствует выгрузке Конфигуратора' };
        return { chain: [P.agent, P.designer, P.ibcmd], config: ['connection', 'source-set[]'], note: 'выгрузка агента соответствует выгрузке Конфигуратора' };
      }
    },
    {
      id: 'upload', verb: 'upload', title: 'Загрузить .cf / .cfe в базу',
      what: 'Применяет пакет к основной конфигурации, целиком заменяя её; --mode combine или update — по правилам платформы.',
      cmd: function (ctx) { return (ctx.type === 'EXTENSION' ? 'v8-runner upload ext.cfe --ref my-ext' : 'v8-runner upload main.cf'); },
      applies: function (ctx) { return notExternal(ctx, 'upload'); },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { chain: [P.designer], config: ['connection'], note: 'по прямому шлюзу; через SSH-шлюз нет: в его наборе нет сравнения, а проба совместимости перед загрузкой обязательна' };
        return { chain: [P.agent, P.designer], config: ['connection'], note: '' };
      }
    },
    {
      id: 'make', verb: 'make', title: 'Собрать пакет из исходников',
      what: 'Собирает .cf, .cfe, .epf, .erf из исходников; база не нужна.',
      cmd: function (ctx) { return (ctx.type === 'EXTERNAL' ? 'v8-runner make --output .build/epf' : ctx.type === 'EXTENSION' ? 'v8-runner make my-ext --output .build/ext.cfe' : 'v8-runner make --output .build/main.cf'); },
      applies: function (ctx) { return null; },
      target: function (ctx) {
        // Пакет собирается из исходников без базы: ibcmd config import --out. Конфигуратор — запасной путь через временную базу.
        return { chain: [P.ibcmd, P.rs, P.designer], config: ['source-set[]'], note: 'база не нужна: ibcmd собирает пакет из XML, ibcmd-rs — без платформы; Конфигуратор — через временную базу' };
      }
    },
    {
      id: 'download', verb: 'download', title: 'Забрать конфигурацию базы в пакет .cf / .cfe',
      what: 'Забирает конфигурацию базы в пакет: основную или, с --state db, конфигурацию базы данных.',
      cmd: function (ctx) { return (ctx.type === 'EXTENSION' ? 'v8-runner download my-ext --output ext.cfe' : 'v8-runner download --output main.cf'); },
      applies: function (ctx) { return notExternal(ctx, 'экспорт конфигурации'); },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { chain: [P.designer, P.agent], config: ['connection'], note: '--state db — только Конфигуратор по прямому шлюзу; dump-cfg по SSH отдаёт основную' };
        if (ctx.target === 'cluster') return { chain: [P.agent, P.designer], config: ['connection'], note: '' };
        return { chain: [P.agent, P.designer, P.ibcmd], config: ['connection'], note: '' };
      }
    },
    {
      id: 'ib-dump', verb: 'infobase dump / restore', title: 'Снять и вернуть всю базу (.dt)',
      what: 'Снимает базу в .dt и возвращает обратно.',
      cmd: function (ctx) { return 'v8-runner infobase dump --output ib.dt'; },
      applies: function (ctx) { return notExternal(ctx, 'снимок базы'); },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { chain: [P.designer], config: ['connection'], note: '/DumpIB и /RestoreIB по прямому шлюзу; через SSH-шлюз намеренно нет: dump-ib роняет ibsrv 8.3.27 (замер)' };
        return { chain: [P.agent, P.designer], config: ['connection'], note: '' };
      }
    },
    {
      id: 'extensions', verb: 'extensions', title: 'Расширения: состав и свойства',
      what: 'Свойства расширений и их состав в базе.',
      cmd: function (ctx) { return 'v8-runner extensions list'; },
      applies: function (ctx) { return notExternal(ctx, 'extensions'); },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { chain: [P.agent, P.designer], config: ['standalone.gate', 'connection — для имён'], note: 'свойства — группой config extensions по SSH-шлюзу; имена — /DumpDBCfgList по прямому шлюзу' };
        if (ctx.target === 'cluster') return { chain: [P.agent, P.designer], config: ['connection'], note: 'Конфигуратор перечислит имена (/DumpDBCfgList), свойства — только агент; ibcmd не использует кластерную строку connection' };
        return { chain: [P.ibcmd, P.agent], config: ['connection'], note: 'состав и свойства — ibcmd; агент — по ключу' };
      }
    },
    {
      id: 'check', verb: 'check', title: 'Проверить конфигурацию и модули',
      what: 'Проверяет целостность, ссылки и синтаксис по режимам; для EDT — проект.',
      cmd: function (ctx) { return 'v8-runner check'; },
      applies: function (ctx) { return ctx.format === 'EDT' ? needEdt(ctx) : (ctx.type === 'EXTERNAL' ? { kind: 'subject', why: 'проверка конфигурации в XML-режиме не применяется к внешним обработкам и отчётам', fix: '' } : null); },
      target: function (ctx) {
        if (ctx.format === 'EDT') return { chain: [P.edt], config: ['tools.edt_cli.*'], note: 'validate проекта; база не нужна' };
        if (ctx.target === 'cluster') return { chain: [P.designer], config: ['connection'], note: '/CheckConfig со всеми режимами' };
        if (ctx.target === 'standalone') return { chain: [P.designer], config: ['connection'], note: '/CheckConfig по прямому шлюзу; в наборе SSH-шлюза проверок нет' };
        return { chain: [P.designer], config: ['connection'], note: '/CheckConfig со всеми режимами выполняет Конфигуратор' };
      }
    },
    {
      id: 'publish', verb: 'publish', title: 'Опубликовать базу на веб-сервере',
      cmd: function (ctx) { return 'v8-runner web publish'; },
      what: 'Публикует базу на Apache или IIS, чтобы открыть её веб-клиентом.',
      applies: function (ctx) {
        if (ctx.target === 'standalone') return { kind: 'subject', why: 'автономный сервер отдаёт HTTP сам', fix: 'публикация не нужна' };
        return null;
      },
      target: function (ctx) {
        return { chain: [P.webinst], config: ['connection', 'web.*'],
                 note: 'нужны права администратора; каталог публикации должен существовать' };
      }
    },
    {
      id: 'launch', verb: 'launch', title: 'Запустить клиент или Конфигуратор',
      what: 'Запускает клиент или Конфигуратор.',
      cmd: function (ctx) { return 'v8-runner launch thin'; },
      applies: function (ctx) { return null; },
      target: function (ctx) {
        // У цели два адреса, и тонкий клиент открывается любым; умолчание задаёт вид цели.
        // Клиент запускается локально в любом случае — платформа нужна и для веб-пути.
        if (ctx.target === 'standalone') {
          return { chain: [P.client], config: ['connection', 'web.url — для --via web', 'tools.enterprise.additional-launch-keys (необязательно)'],
                   note: 'designer, thin и mcp по прямому шлюзу; thick и ordinary отказывают: прямой шлюз толстого клиента не пускает, обычное приложение сервер не поддерживает; --via web ведёт тонкий клиент по HTTP сервера' };
        }
        return { chain: [P.client], config: ['connection', 'web.url — для --via web', 'tools.enterprise.additional-launch-keys (необязательно)'],
                 note: 'умолчание — строка подключения; --via web открывает ту же базу по опубликованному адресу ws-соединением' };
      }
    },
    {
      id: 'launch-web', verb: 'launch web', title: 'Открыть опубликованную базу в браузере',
      cmd: function (ctx) { return 'v8-runner launch web'; },
      what: 'Открывает базу веб-клиентом по адресу публикации.',
      applies: function (ctx) { return null; },
      target: function (ctx) {
        return { chain: [P.browser], config: ['web.url'],
                 note: ctx.target === 'standalone' ? 'адрес даёт сам автономный сервер' : 'web.url — адрес готовой публикации; управлять публикацией для открытия клиента не требуется' };
      }
    },
    {
      id: 'convert', verb: 'convert', title: 'Перевести исходники между форматами',
      what: 'Переводит исходники между EDT и XML.',
      cmd: function (ctx) { return 'v8-runner convert'; },
      applies: function (ctx) { return null; },
      target: function (ctx) {
        return { chain: [P.edt, P.ibcmd, P.rs], config: ['format', 'source-set[]'], note: 'EDT ↔ XML делает 1cedtcli; пакет ↔ XML — ibcmd или ibcmd-rs без базы' };
      }
    }
  ];

  function compute(ctx) {
    ctx.tools = Object.assign({ browser: true }, ctx.tools);
    return SCENARIOS.map(function (s) {
      var blocked = s.applies(ctx);
      var kind = blocked ? blocked.kind : null;

      var r = s.target(ctx);
      var chain = r.chain || [];
      if (ctx.target === 'cluster' && chain.indexOf(P.ibcmd) !== -1 && ['make', 'convert'].indexOf(s.id) === -1) {
        r.note = 'ibcmd — только после остановки серверных процессов кластера, обслуживающего эту ИБ. Отсутствия сеансов недостаточно; использование с работающим кластером может разрушить базу. ' + (r.note || '');
      }
      var selected = chain.length ? firstReady(chain, ctx) : null;

      var status = 'ok', why = '', fix = '';
      if (kind) { status = kind; why = blocked.why; fix = blocked.fix || ''; }
      else if (chain.length && !selected) {
        status = 'tool';
        why = 'нет ни одного из: ' + chain.map(function (p) { return missingFor(p, ctx); }).join(', ');
        fix = '';
      }

      return {
        s: s, status: status, why: why, fix: fix,
        chain: chain, selected: selected,
        config: r.config || [], note: r.note || '',
        noProvider: chain.length === 0
      };
    });
  }

  // Порядок первого запуска для случая «исходники есть, базы нет». Обратный случай —
  // одна команда clone. У внешних обработок базы нет, поэтому и шагов с базой у них нет.
  function FIRST_RUN(ctx) {
    if (ctx.type === 'EXTERNAL') return ['init', 'make', 'convert'];
    return ['init', 'infobase-create', 'push', 'check', 'test'];
  }

  return { AXES: AXES, SCENARIOS: SCENARIOS, PROVIDERS: P, compute: compute, FIRST_RUN: FIRST_RUN };
})();
