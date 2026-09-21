// Данные конфигуратора сценариев. Факты — из docs/CAPABILITIES.md, docs/CONFIGURATION.md,
// реестра spec/arch и замеров 12–14.09.2026. Всё, чего здесь нет, конфигуратор не обещает.
//
// Сайт описывает целевое состояние и читает только ветку target().
// Ветка today() описывает то, что раннер делает сейчас, и оставлена намеренно:
// из разницы today() и target() механически собирается список работ.
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
      { id: 'standalone', label: 'автономный сервер', hint: 'секция infobase.standalone; Конфигуратор идёт в прямой шлюз как в кластер, SSH-шлюз сервера даёт агентский набор без платформы у раннера' }
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
  // today(ctx) → {chain:[P…], config:[…], note} по текущему коду (матрица провайдеров);
  // target(ctx) → то же по DEC.2026-09-14.PROVIDER-CHOSEN-PER-OPERATION (цепочка умолчаний и матрица).
  // Три разные причины, по которым сценарий сейчас не отработает. Их нельзя смешивать:
  // subject — предмет не тот (навсегда); tool — нет инструмента в окружении (поставьте — заработает);
  // soon — раннер пока не умеет (уже решено, ещё не реализовано).
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
  function builderChoice(ctx, designerOk, ibcmdOk) {
    // сегодня: цепочка умолчаний из матрицы; показываем оба варианта, если оба возможны
    var out = [];
    if (designerOk && ctx.tools.designer) out.push(P.designer);
    if (ibcmdOk && ctx.tools.ibcmd) out.push(P.ibcmd);
    return out;
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
      today: function (ctx) { return { chain: [], config: [], note: 'платформу не запускает и базу не трогает: это первая команда, когда непонятно' }; },
      target: function (ctx) { return this.today(ctx); }
    },
    {
      id: 'init', verb: 'init', title: 'Завести проект здесь',
      what: 'Пишет v8project.yaml по найденным исходникам.',
      cmd: function (ctx) { return 'v8-runner init'; },
      applies: function () { return null; },
      today: function (ctx) { return { chain: [], config: [], note: 'платформа не нужна; тип каждого набора определяется по содержимому файлов, не по именам каталогов' }; },
      target: function (ctx) { return this.today(ctx); }
    },
    {
      id: 'clone', verb: 'clone', title: 'Завести проект из существующей базы',
      what: 'Привязывает каталог к базе и делает первую выгрузку.',
      cmd: function (ctx) { return 'v8-runner clone --from "File=/srv/ib/demo"'; },
      applies: function (ctx) { return notExternal(ctx, 'clone'); },
      today: function (ctx) { return { chain: builderChoice(ctx, true, true), config: [], note: 'сейчас это bootstrap' }; },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { chain: [P.designer, P.agent], config: ['infobase.connection или infobase.standalone.gate'], note: 'пишет v8project.yaml, местный слой и делает pull' };
        return { chain: [P.agent, P.designer], config: [], note: 'пишет v8project.yaml, местный слой и делает pull' };
      }
    },
    {
      id: 'infobase-create', verb: 'infobase create', title: 'Создать базу',
      what: 'Создаёт базу, если её нет.',
      cmd: function (ctx) { return 'v8-runner infobase create'; },
      applies: function (ctx) { return standaloneRefuses(ctx, 'базу автономного сервера создают ibcmd до запуска сервера, на его машине; раннер к нему подключается, ничего не запуская') || needEdt(ctx); },
      today: function (ctx) {
        var chain = builderChoice(ctx, ctx.target === 'file', true);
        var cfg = ['infobase.connection'];
        if (ctx.target === 'cluster') cfg.push('infobase.dbms — чтобы создать базу');
        var note = ctx.target === 'cluster' ? 'создаёт ibcmd по данным СУБД; Designer серверную базу не создаёт' : 'файловую базу создаёт любой из двух';
        return { chain: chain, config: cfg, note: note };
      },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { kind: 'subject', why: 'базу автономного сервера создают до его запуска, на его машине', fix: 'ibcmd server config init, затем ibcmd infobase create --load|--import|--restore; раннер подключается к уже работающему шлюзу' };
        if (ctx.target === 'cluster') return { chain: [P.designer, P.rac], config: ['infobase.connection', 'infobase.dbms.*', 'infobase.cluster.user — если в кластере заведены администраторы'], note: 'CREATEINFOBASE с клиент-серверной строкой: регистрация в кластере и CrSQLDB=Y одной командой; rac — запасной путь' };
        return { chain: [P.ibcmd, P.designer], config: ['infobase.connection'], note: 'ibcmd создаёт файловую базу сразу с конфигурацией из исходников (--import)' };
      }
    },
    {
      id: 'push', verb: 'push', title: 'Отправить исходники в базу',
      what: 'Отправляет изменённые исходники в основную конфигурацию и применяет к базе данных; --no-apply останавливается раньше.',
      cmd: function (ctx) { return 'v8-runner push'; },
      applies: function (ctx) { return notExternal(ctx, 'push') || needEdt(ctx); },
      today: function (ctx) {
        var chain = builderChoice(ctx, true, true);
        var cfg = ['infobase.connection', 'source-set[]', 'push.partialLoadThreshold (необязательно)'];
        
        return { chain: chain, config: cfg, note: ctx.format === 'EDT' ? 'шаг экспорта EDT → платформа, затем загрузка сгенерированного' : 'partial или full решает обнаружение изменений' };
      },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { chain: [P.designer, P.agent], config: ['infobase.connection — прямой шлюз', 'infobase.standalone.gate и exchange — для agent', 'source-set[]'], note: 'Конфигуратор по прямому шлюзу читает исходники у раннера; agent — load-config-from-files + update-db-cfg одной сессией, файлы через канал обмена' };
        if (ctx.target === 'cluster') return { chain: [P.agent, P.designer], config: ['infobase.connection', 'source-set[]'], note: 'одна сессия агента на всю команду; без агента — Designer' };
        return { chain: [P.agent, P.designer, P.ibcmd], config: ['infobase.connection', 'source-set[]'], note: 'одна сессия агента на всю команду; без агента — Designer' };
      }
    },
    {
      id: 'apply', verb: 'apply', title: 'Применить к базе данных',
      what: 'Приводит конфигурацию базы данных к основной; --sessions force разрешает завершать чужие сеансы.',
      cmd: function (ctx) { return 'v8-runner apply'; },
      applies: function (ctx) { return notExternal(ctx, 'apply') || needEdt(ctx); },
      today: function (ctx) { return { chain: builderChoice(ctx, true, true), config: ['infobase.connection'], note: 'часть build; отдельной команды нет' }; },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { chain: [P.designer, P.agent], config: ['infobase.connection', 'infobase.standalone.gate — для agent'], note: '/UpdateDBCfg по прямому шлюзу; agent — update-db-cfg по SSH' };
        if (ctx.target === 'cluster') return { chain: [P.agent, P.designer], config: ['infobase.connection'], note: '/UpdateDBCfg' };
        return { chain: [P.agent, P.designer, P.ibcmd], config: ['infobase.connection'], note: '/UpdateDBCfg или ibcmd config apply' };
      }
    },
    {
      id: 'diff', verb: 'diff', title: 'Что изменилось в базе',
      what: 'Перечисляет объекты, изменившиеся в базе с последней синхронизации; --against сравнивает с базой данных, поставщиком или пакетом.',
      cmd: function (ctx) { return 'v8-runner diff'; },
      applies: function (ctx) { return notExternal(ctx, 'diff') || needEdt(ctx); },
      today: function (ctx) { return { chain: [], config: [], note: 'нет' }; },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { chain: [P.designer, P.agent], config: ['infobase.connection'], note: '/CompareCfg и /DumpConfigToFiles -getChanges по прямому шлюзу; agent — только по файлу версий, отчёта сравнения у SSH-шлюза нет' };
        if (ctx.target === 'cluster') return { chain: [P.designer], config: ['infobase.connection'], note: '/DumpConfigToFiles -getChanges по файлу версий; отчёты сравнения — /CompareCfg' };
        return { chain: [P.ibcmd, P.designer], config: ['infobase.connection'], note: 'ibcmd export status по файлу версий; отчёты сравнения — /CompareCfg' };
      }
    },
    {
      id: 'sessions', verb: 'sessions', title: 'Сеансы: список, завершение, блокировка',
      what: 'Окно обслуживания: запретить новые сеансы, завершить старые, после применения разрешить.',
      cmd: function (ctx) { return 'v8-runner sessions terminate --all'; },
      applies: function (ctx) { return ctx.target === 'file' ? { kind: 'target', why: 'у файловой базы нет сервера, который ведёт сеансы', fix: 'веб-сеансы завершает apply --sessions force в момент применения' } : notExternal(ctx, 'sessions'); },
      today: function (ctx) { return { chain: [], config: [], note: 'нет' }; },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { chain: [P.ibcmd], config: ['infobase.standalone.*'], note: 'ibcmd session list и terminate; блокировки начала сеансов у автономного сервера нет' };
        return { chain: [P.rac], config: ['infobase.cluster.ras', 'infobase.cluster.user и password — администратор кластера', 'infobase.user и password — для deny и allow'], note: 'rac session и rac infobase update --sessions-deny; исполнитель один, ключа нет' };
      }
    },
    {
      id: 'test', verb: 'test', title: 'Прогнать тесты',
      what: 'Запускает YAxUnit или Vanessa.',
      cmd: function (ctx) { return 'v8-runner test yaxunit all'; },
      applies: function (ctx) { return notExternal(ctx, 'test') || needEdt(ctx); },
      today: function (ctx) { return { chain: [P.client], config: ['tests.yaxunit.* или tests.va.*', 'tools.va.epf_path — для Vanessa'], note: 'сначала отправка, как у push; test --no-build её пропускает' }; },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { chain: [P.client], config: ['infobase.connection', 'infobase.web.url — для --via web', 'tests.yaxunit.* или tests.va.*'], note: 'тонкий клиент по прямому шлюзу или по HTTP с --via web; тесты в толстом клиенте недоступны — прямой шлюз его не пускает; сначала push, --no-push пропускает' };
        return { chain: [P.client], config: ['infobase.connection', 'tests.yaxunit.* или tests.va.*', 'tools.va.epf_path — для Vanessa'], note: 'сначала push, затем прогон; --no-push пропускает отправку' };
      }
    },
    {
      id: 'pull', verb: 'pull', title: 'Забрать изменения из базы',
      what: 'Выгружает конфигурацию базы в исходники.',
      cmd: function (ctx) { return 'v8-runner pull'; },
      applies: function (ctx) { return notExternal(ctx, 'pull') || needEdt(ctx); },
      today: function (ctx) {
        var chain = builderChoice(ctx, true, true);
        return { chain: chain, config: ['infobase.connection', 'source-set[]'], note: 'у ibcmd режим partial деградирует в incremental с предупреждением; публикация через staging и backup' };
      },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { chain: [P.designer, P.agent], config: ['infobase.connection', 'source-set[]', 'infobase.standalone.gate и exchange — для agent'], note: 'Конфигуратор пишет выгрузку у раннера; agent — dump-config-to-files по SSH, результат через объявленный канал обмена' };
        if (ctx.target === 'cluster') return { chain: [P.agent, P.designer], config: ['infobase.connection', 'source-set[]'], note: 'выгрузка агента побайтно равна выгрузке Конфигуратора (замер)' };
        return { chain: [P.agent, P.designer, P.ibcmd], config: ['infobase.connection', 'source-set[]'], note: 'выгрузка агента побайтно равна выгрузке Конфигуратора (замер)' };
      }
    },
    {
      id: 'infobase-load', verb: 'infobase load', title: 'Загрузить .cf / .cfe в базу',
      what: 'Применяет пакет к основной конфигурации, целиком заменяя её; --mode combine или update — по правилам платформы.',
      cmd: function (ctx) { return (ctx.type === 'EXTENSION' ? 'v8-runner infobase load ext.cfe --ref my-ext' : 'v8-runner infobase load main.cf'); },
      applies: function (ctx) { return notExternal(ctx, 'infobase load'); },
      today: function (ctx) { return { chain: ctx.tools.designer ? [P.designer] : [], config: ['infobase.connection'], note: 'только Конфигуратор; состояния совместимости supported / absent / not_established / not_probed' }; },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { chain: [P.designer], config: ['infobase.connection'], note: 'по прямому шлюзу; через SSH-шлюз нет: в его наборе нет сравнения, а проба совместимости перед загрузкой обязательна' };
        return { chain: [P.agent, P.designer], config: ['infobase.connection'], note: '' };
      }
    },
    {
      id: 'make', verb: 'make', title: 'Собрать пакет из исходников',
      what: 'Собирает .cf, .cfe, .epf, .erf из исходников; база не нужна.',
      cmd: function (ctx) { return (ctx.type === 'EXTERNAL' ? 'v8-runner make --output build/epf' : ctx.type === 'EXTENSION' ? 'v8-runner make my-ext --output build/ext.cfe' : 'v8-runner make --output build/main.cf'); },
      applies: function (ctx) { return null; },
      today: function (ctx) {
        if (ctx.type === 'EXTERNAL') return { chain: ctx.tools.designer ? [P.designer] : [], config: ['source-set[] с type EXTERNAL_*'], note: 'внешние собираются Конфигуратором из XML; базы не касается' };
        return { chain: ctx.tools.designer ? [P.designer] : [], config: ['infobase.connection'], note: 'только Конфигуратор' };
      },
      target: function (ctx) {
        // Пакет собирается из исходников без базы: ibcmd config import --out. Конфигуратор — запасной путь через временную базу.
        return { chain: [P.ibcmd, P.designer], config: ['source-set[]'], note: 'база не нужна: ibcmd собирает пакет из XML; Конфигуратор — через временную базу' };
      }
    },
    {
      id: 'infobase-download', verb: 'infobase download', title: 'Забрать конфигурацию базы в пакет .cf / .cfe',
      what: 'Забирает конфигурацию базы в пакет: основную или, с --state db, конфигурацию базы данных.',
      cmd: function (ctx) { return (ctx.type === 'EXTENSION' ? 'v8-runner infobase download my-ext --output ext.cfe' : 'v8-runner infobase download --output main.cf'); },
      applies: function (ctx) { return notExternal(ctx, 'экспорт конфигурации'); },
      today: function (ctx) {
        var chain = builderChoice(ctx, true, ctx.target === 'file' || ctx.target === 'cluster');
        return { chain: chain, config: ['infobase.connection'].concat(ctx.target === 'cluster' ? ['infobase.dbms.* — для ibcmd'] : []), note: 'раннер берёт первого готового из цепочки; квитанция называет пропущенных' };
      },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { chain: [P.designer, P.agent], config: ['infobase.connection'], note: '--state db — только Конфигуратор по прямому шлюзу; dump-cfg по SSH отдаёт основную' };
        if (ctx.target === 'cluster') return { chain: [P.agent, P.designer], config: ['infobase.connection'], note: '' };
        return { chain: [P.agent, P.designer, P.ibcmd], config: ['infobase.connection'], note: '' };
      }
    },
    {
      id: 'ib-dump', verb: 'infobase dump / restore', title: 'Снять и вернуть всю базу (.dt)',
      what: 'Снимает базу в .dt и возвращает обратно.',
      cmd: function (ctx) { return 'v8-runner infobase dump --output ib.dt'; },
      applies: function (ctx) { return notExternal(ctx, 'снимок базы'); },
      today: function (ctx) { return { chain: ctx.tools.designer ? [P.designer] : [], config: ['infobase.connection'], note: 'ibcmd для .dt остаётся экспериментом, пока нет проверки эксклюзивного доступа' }; },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { chain: [P.designer], config: ['infobase.connection'], note: '/DumpIB и /RestoreIB по прямому шлюзу; через SSH-шлюз намеренно нет: dump-ib роняет ibsrv 8.3.27 (замер)' };
        return { chain: [P.agent, P.designer], config: ['infobase.connection'], note: '' };
      }
    },
    {
      id: 'extensions', verb: 'extensions', title: 'Расширения: состав и свойства',
      what: 'Свойства расширений и их состав в базе.',
      cmd: function (ctx) { return 'v8-runner extensions list'; },
      applies: function (ctx) { return notExternal(ctx, 'extensions'); },
      today: function (ctx) { return { chain: ctx.tools.ibcmd ? [P.ibcmd] : [], config: ['infobase.connection'].concat(ctx.target === 'cluster' ? ['infobase.dbms.*'] : []), note: 'состав базы умеет только ibcmd: у Конфигуратора нет пакетного списка' }; },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { chain: [P.agent, P.designer], config: ['infobase.standalone.gate', 'infobase.connection — для имён'], note: 'свойства — группой config extensions по SSH-шлюзу; имена — /DumpDBCfgList по прямому шлюзу' };
        if (ctx.target === 'cluster') return { chain: [P.agent, P.designer], config: ['infobase.connection'], note: 'Конфигуратор перечислит имена (/DumpDBCfgList), свойства — только агент; ibcmd к базе под кластером не применяется' };
        return { chain: [P.agent, P.ibcmd], config: ['infobase.connection'], note: '' };
      }
    },
    {
      id: 'check', verb: 'check', title: 'Проверить конфигурацию и модули',
      what: 'Проверяет целостность, ссылки и синтаксис по режимам; для EDT — проект.',
      cmd: function (ctx) { return 'v8-runner check'; },
      applies: function (ctx) { return ctx.format === 'EDT' ? needEdt(ctx) : (ctx.type === 'EXTERNAL' ? { kind: 'subject', why: 'для внешних наборов не описана', fix: '' } : null); },
      today: function (ctx) { return ctx.format === 'EDT' ? { chain: [P.edt], config: ['tools.edt_cli.*'], note: 'validate; одна общая сессия EDT при interactive-mode=true' } : { chain: ctx.tools.designer ? [P.designer] : [], config: ['infobase.connection'], note: 'вердикт по коду выхода: 0 чисто, 101 есть замечания; журнал переносится как улика' }; },
      target: function (ctx) {
        if (ctx.format === 'EDT') return { chain: [P.edt], config: ['tools.edt_cli.*'], note: 'validate проекта; база не нужна' };
        if (ctx.target === 'cluster') return { chain: [P.designer], config: ['infobase.connection'], note: '/CheckConfig со всеми режимами' };
        if (ctx.target === 'standalone') return { chain: [P.designer], config: ['infobase.connection'], note: '/CheckConfig по прямому шлюзу; в наборе SSH-шлюза проверок нет' };
        return { chain: [P.designer], config: ['infobase.connection'], note: '/CheckConfig со всеми режимами; у ibcmd состав проверки не описан' };
      }
    },
    {
      id: 'publish', verb: 'publish', title: 'Опубликовать базу на веб-сервере',
      cmd: function (ctx) { return 'v8-runner publish'; },
      what: 'Публикует базу на Apache или IIS, чтобы открыть её веб-клиентом.',
      applies: function (ctx) {
        if (ctx.target === 'standalone') return { kind: 'subject', why: 'автономный сервер отдаёт HTTP сам', fix: 'публикация не нужна' };
        return null;
      },
      today: function (ctx) { return this.target(ctx); },
      target: function (ctx) {
        return { chain: [P.webinst], config: ['infobase.connection', 'infobase.web.*'],
                 note: 'нужны права администратора; каталог публикации должен существовать' };
      }
    },
    {
      id: 'launch', verb: 'launch', title: 'Запустить клиент или Конфигуратор',
      what: 'Запускает клиент или Конфигуратор.',
      cmd: function (ctx) { return 'v8-runner launch thin'; },
      applies: function (ctx) { return null; },
      today: function (ctx) { return this.target(ctx); },
      target: function (ctx) {
        // У цели два адреса, и тонкий клиент открывается любым; умолчание задаёт вид цели.
        // Клиент запускается локально в любом случае — платформа нужна и для веб-пути.
        if (ctx.target === 'standalone') {
          return { chain: [P.client], config: ['infobase.connection', 'infobase.web.url — для --via web', 'tools.enterprise.additional-launch-keys (необязательно)'],
                   note: 'designer, thin и mcp по прямому шлюзу; thick и ordinary отказывают: прямой шлюз толстого клиента не пускает, обычное приложение сервер не поддерживает; --via web ведёт тонкий клиент по HTTP сервера' };
        }
        return { chain: [P.client], config: ['infobase.connection', 'infobase.web.url — для --via web', 'tools.enterprise.additional-launch-keys (необязательно)'],
                 note: 'умолчание — строка подключения; --via web открывает ту же базу по опубликованному адресу ws-соединением' };
      }
    },
    {
      id: 'launch-web', verb: 'launch web', title: 'Открыть опубликованную базу в браузере',
      cmd: function (ctx) { return 'v8-runner launch web'; },
      what: 'Открывает базу веб-клиентом по адресу публикации.',
      applies: function (ctx) {
        if (ctx.target === 'standalone') return null;
        if (!ctx.tools.web) return { kind: 'tool', why: 'база не опубликована', fix: 'нужен веб-сервер и команда publish' };
        return null;
      },
      today: function (ctx) { return this.target(ctx); },
      target: function (ctx) {
        return { chain: [P.browser], config: ['infobase.web.url'],
                 note: ctx.target === 'standalone' ? 'адрес даёт сам автономный сервер' : 'адрес появляется после publish' };
      }
    },
    {
      id: 'convert', verb: 'convert', title: 'Перевести исходники между форматами',
      what: 'Переводит исходники между EDT и XML.',
      cmd: function (ctx) { return 'v8-runner convert'; },
      applies: function (ctx) { return null; },
      today: function (ctx) { return ctx.tools.edt ? { chain: [P.edt], config: ['format', 'source-set[]', 'tools.edt_cli.path'], note: 'только между EDT и XML; только CLI, в MCP не публикуется' } : { chain: [], config: [], note: 'нет' }; },
      target: function (ctx) {
        return { chain: [P.edt, P.ibcmd, P.rs], config: ['format', 'source-set[]'], note: 'EDT ↔ XML делает 1cedtcli; пакет ↔ XML — ibcmd или ibcmd-rs без базы' };
      }
    }
  ];

  function compute(ctx, mode) {
    ctx.mode = mode;
    ctx.tools = Object.assign({ browser: true }, ctx.tools);
    return SCENARIOS.map(function (s) {
      var blocked = s.applies(ctx);
      var kind = blocked ? blocked.kind : null;

      // Что раннер делает, всегда считаем — даже если сейчас не отработает: читатель пришёл за этим.
      // Для «пока не умеет» показываем целевую ветку: так видно, чем это будет сделано.
      var branch = (kind === 'soon' || mode === 'target') ? s.target : s.today;
      var r = branch.call(s, ctx);
      var chain = r.chain || [];
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
