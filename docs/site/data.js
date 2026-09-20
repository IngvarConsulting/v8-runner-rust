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
      { id: 'EXTERNAL',      label: 'внешние', hint: 'обработки и отчёты: в базу не грузятся, собираются в .epf и .erf' }
    ],
    target: [
      { id: 'file',       label: 'файловая',  hint: 'File=…; раннер может создать её сам. Агента для неё поднимает раннер — нужна платформа' },
      { id: 'cluster',    label: 'кластер 1С', hint: 'Srvr=…;Ref=…; данные СУБД нужны, чтобы создать базу, а не чтобы работать с готовой. Агента для неё поднимает раннер — нужна платформа' },
      { id: 'standalone', label: 'автономный сервер', hint: 'секция infobase.standalone; сервер держит свой SSH-шлюз, платформа на машине раннера не нужна' }
    ],
    tools: [
      { id: 'designer', label: 'платформа 1С (1cv8, Конфигуратор)', short: 'платформа 1С', def: true },
      { id: 'ibcmd',    label: 'ibcmd', short: 'ibcmd', def: true },
      { id: 'edt',      label: 'EDT и 1cedtcli', short: 'EDT', def: false },
      { id: 'agent',    label: 'агентский режим', short: 'агент', def: true },
      { id: 'rs',       label: 'ibcmd-rs', short: 'ibcmd-rs', def: false },
      { id: 'web',      label: 'веб-сервер Apache или IIS', short: 'веб-сервер', def: false }
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
      ? { kind: 'subject', why: 'внешние обработки в базу не грузятся', fix: 'для них — make и convert' }
      : null;
  }
  // Автономный сервер отвечает только через свой SSH-шлюз, и набор операций у него
  // закрыт решением, а не недоделкой: в матрице (src/domain/capability.rs) строки есть
  // только у build, dump, make, extensions и infobase configuration export. Остальное
  // отказывает типизированно, и «когда-нибудь появится» тут сказать нельзя.
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
      id: 'config-init', verb: 'config init', title: 'Создать конфигурационный файл',
      what: 'Пишет v8project.yaml по найденным исходникам.',
      cmd: function (ctx) { return 'v8-runner config init'; },
      applies: function () { return null; },
      today: function (ctx) { return { chain: [], config: [], note: 'платформа не нужна; тип каждого набора определяется по содержимому файлов, не по именам каталогов' }; },
      target: function (ctx) { return this.today(ctx); }
    },
    {
      id: 'init', verb: 'init', title: 'Подготовить базу',
      what: 'Создаёт базу, если её нет.',
      cmd: function (ctx) { return 'v8-runner init'; },
      applies: function (ctx) { return standaloneRefuses(ctx, 'у автономного сервера базу не создают снаружи: раннер к нему подключается, ничего не запуская') || needEdt(ctx); },
      today: function (ctx) {
        var chain = builderChoice(ctx, ctx.target === 'file', true);
        var cfg = ['infobase.connection'];
        if (ctx.target === 'cluster') cfg.push('infobase.dbms — чтобы создать базу');
        var note = ctx.target === 'cluster' ? 'создаёт ibcmd по данным СУБД; Designer серверную базу не создаёт' : 'файловую базу создаёт любой из двух';
        return { chain: chain, config: cfg, note: note };
      },
      target: function (ctx) {
        var chain = [P.ibcmd, P.designer];
        if (ctx.target === 'standalone') return { kind: 'subject', why: 'автономный сервер поднимает человек', fix: 'раннер его не создаёт и не запускает: infobase.standalone называет уже работающий шлюз' };
        return { chain: chain, config: ['infobase.connection'].concat(ctx.target === 'cluster' ? ['infobase.dbms.*'] : []), note: 'цепочка: ibcmd, затем Designer' };
      }
    },
    {
      id: 'build', verb: 'build', title: 'Загрузить изменения в базу',
      what: 'Грузит изменённые исходники в базу.',
      cmd: function (ctx) { return 'v8-runner build'; },
      applies: function (ctx) { return notExternal(ctx, 'build') || needEdt(ctx); },
      today: function (ctx) {
        var chain = builderChoice(ctx, true, true);
        var cfg = ['infobase.connection', 'source-set[]', 'build.partialLoadThreshold (необязательно)'];
        
        return { chain: chain, config: cfg, note: ctx.format === 'EDT' ? 'шаг экспорта EDT → платформа, затем загрузка сгенерированного' : 'partial или full решает обнаружение изменений' };
      },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { chain: [P.agent], config: ['infobase.standalone.*'], note: 'load-config-from-files + update-db-cfg в одной сессии шлюза' };
        return { chain: [P.agent, P.designer], config: ['infobase.connection', 'source-set[]'], note: 'одна сессия агента на всю команду; без агента — Designer' };
      }
    },
    {
      id: 'test', verb: 'test', title: 'Прогнать тесты',
      what: 'Запускает YAxUnit или Vanessa.',
      cmd: function (ctx) { return 'v8-runner test yaxunit all'; },
      applies: function (ctx) { return notExternal(ctx, 'test') || standaloneRefuses(ctx, 'тест запускает клиента по строке подключения, которой у этой цели нет') || needEdt(ctx); },
      today: function (ctx) { return { chain: [P.client], config: ['tests.yaxunit.* или tests.va.*', 'tools.va.epf_path — для Vanessa'], note: 'провайдер сборки — как у build; test --no-build пропускает сборку' }; },
      target: function (ctx) { return this.today(ctx); }
    },
    {
      id: 'dump', verb: 'dump', title: 'Выгрузить базу в исходники',
      what: 'Выгружает конфигурацию базы в исходники.',
      cmd: function (ctx) { return 'v8-runner dump --mode full'; },
      applies: function (ctx) { return notExternal(ctx, 'dump') || needEdt(ctx); },
      today: function (ctx) {
        var chain = builderChoice(ctx, true, true);
        return { chain: chain, config: ['infobase.connection', 'source-set[]'], note: 'у ibcmd режим partial деградирует в incremental с предупреждением; публикация через staging и backup' };
      },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { chain: [P.agent], config: ['infobase.standalone.gate', 'infobase.standalone.exchange'], note: 'dump-config-to-files по шлюзу; результат забирается объявленным каналом — каталогом пользователя шлюза или SFTP того же соединения' };
        return { chain: [P.agent, P.designer], config: ['infobase.connection', 'source-set[]'], note: 'выгрузка агента побайтно равна выгрузке Конфигуратора' };
      }
    },
    {
      id: 'load', verb: 'load', title: 'Загрузить .cf / .cfe в базу',
      what: 'Грузит готовый .cf или .cfe в базу.',
      cmd: function (ctx) { return (ctx.type === 'EXTENSION' ? 'v8-runner load --path ext.cfe --extension ИмяРасширения' : 'v8-runner load --path main.cf'); },
      applies: function (ctx) { return notExternal(ctx, 'load') || standaloneRefuses(ctx, 'у шлюза нет compare-cfg, поэтому загрузку артефакта он не исполняет'); },
      today: function (ctx) { return { chain: ctx.tools.designer ? [P.designer] : [], config: ['infobase.connection'], note: 'только Конфигуратор; состояния совместимости supported / absent / not_established / not_probed' }; },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { kind: 'subject', why: 'у шлюза нет compare-cfg', fix: 'проба совместимости перед загрузкой обязательна; загружайте через build из исходников' };
        return { chain: [P.agent, P.designer], config: ['infobase.connection'], note: '' };
      }
    },
    {
      id: 'make', verb: 'make / artifacts', title: 'Собрать артефакты',
      what: 'Собирает .cf, .cfe, .epf, .erf.',
      cmd: function (ctx) { return (ctx.type === 'EXTERNAL' ? 'v8-runner make --output build/epf' : ctx.type === 'EXTENSION' ? 'v8-runner make --output build/ext.cfe --extension ИмяРасширения' : 'v8-runner make --output build/main.cf'); },
      applies: function (ctx) { return null; },
      today: function (ctx) {
        if (ctx.type === 'EXTERNAL') return { chain: ctx.tools.designer ? [P.designer] : [], config: ['source-set[] с type EXTERNAL_*'], note: 'внешние собираются Конфигуратором из XML; базы не касается' };
        return { chain: ctx.tools.designer ? [P.designer] : [], config: ['infobase.connection'], note: 'только Конфигуратор' };
      },
      target: function (ctx) {
        if (ctx.type === 'EXTERNAL') return { chain: [P.designer, P.rs], config: ['source-set[]'], note: 'ibcmd-rs собирает epf/erf без платформы — пока эксперимент, не умолчание' };
        if (ctx.target === 'standalone') return { chain: [P.agent], config: ['infobase.standalone.*'], note: '' };
        return { chain: [P.agent, P.designer], config: ['infobase.connection'], note: '' };
      }
    },
    {
      id: 'ib-export', verb: 'infobase configuration export', title: 'Сохранить конфигурацию базы в .cf / .cfe',
      what: 'Сохраняет конфигурацию базы в файл.',
      cmd: function (ctx) { return (ctx.type === 'EXTENSION' ? 'v8-runner infobase configuration export --state working --extension ИмяРасширения --output ext.cfe' : 'v8-runner infobase configuration export --state working --output main.cf'); },
      applies: function (ctx) { return notExternal(ctx, 'экспорт конфигурации'); },
      today: function (ctx) {
        var chain = builderChoice(ctx, true, ctx.target === 'file' || ctx.target === 'cluster');
        return { chain: chain, config: ['infobase.connection'].concat(ctx.target === 'cluster' ? ['infobase.dbms.* — для ibcmd'] : []), note: 'раннер берёт первого готового из цепочки; квитанция называет пропущенных' };
      },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { chain: [P.agent], config: ['infobase.standalone.*'], note: 'dump-cfg по шлюзу, только рабочая конфигурация' };
        return { chain: [P.agent, P.ibcmd, P.designer], config: ['infobase.connection'], note: '' };
      }
    },
    {
      id: 'ib-dump', verb: 'infobase dump / restore', title: 'Снять и вернуть всю базу (.dt)',
      what: 'Снимает базу в .dt и возвращает обратно.',
      cmd: function (ctx) { return 'v8-runner infobase dump --output ib.dt'; },
      applies: function (ctx) { return notExternal(ctx, 'снимок базы') || standaloneRefuses(ctx, 'через шлюз снимок не снимают намеренно: dump-ib роняет ibsrv 8.3.27, а restore-ib завершает сеанс сервера — снимок снимают средствами самого сервера'); },
      today: function (ctx) { return { chain: ctx.tools.designer ? [P.designer] : [], config: ['infobase.connection'], note: 'ibcmd для .dt остаётся экспериментом, пока нет проверки эксклюзивного доступа' }; },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { kind: 'subject', why: 'dump-ib через шлюз роняет ibsrv 8.3.27', fix: 'снимок автономного сервера снимают его средствами' };
        return { chain: [P.agent, P.designer], config: ['infobase.connection'], note: '' };
      }
    },
    {
      id: 'extensions', verb: 'extensions', title: 'Расширения: свойства и состав',
      what: 'Свойства расширений и их состав в базе.',
      cmd: function (ctx) { return 'v8-runner extensions list'; },
      applies: function (ctx) { return ctx.type === 'CONFIGURATION' ? { kind: 'subject', why: 'в проекте нет расширений', fix: '' } : notExternal(ctx, 'extensions'); },
      today: function (ctx) { return { chain: ctx.tools.ibcmd ? [P.ibcmd] : [], config: ['infobase.connection'].concat(ctx.target === 'cluster' ? ['infobase.dbms.*'] : []), note: 'состав базы умеет только ibcmd: у Конфигуратора нет пакетного списка' }; },
      target: function (ctx) {
        if (ctx.target === 'standalone') return { chain: [P.agent], config: ['infobase.standalone.*'], note: 'состав и свойства расширений группой config extensions по шлюзу' };
        return { chain: [P.agent, P.ibcmd, P.designer], config: ['infobase.connection'], note: '' };
      }
    },
    {
      id: 'syntax', verb: 'syntax', title: 'Проверить синтаксис',
      what: 'Проверяет синтаксис.',
      cmd: function (ctx) { return (ctx.format === 'EDT' ? 'v8-runner syntax edt' : 'v8-runner syntax designer-config'); },
      applies: function (ctx) { return ctx.format === 'EDT' ? needEdt(ctx) : (ctx.type === 'EXTERNAL' ? { kind: 'subject', why: 'для внешних наборов не описана', fix: '' } : standaloneRefuses(ctx, 'проверка синтаксиса идёт Конфигуратором по строке подключения, которой у этой цели нет')); },
      today: function (ctx) { return ctx.format === 'EDT' ? { chain: [P.edt], config: ['tools.edt_cli.*'], note: 'validate; одна общая сессия EDT при interactive-mode=true' } : { chain: ctx.tools.designer ? [P.designer] : [], config: ['infobase.connection'], note: 'вердикт по коду выхода: 0 чисто, 101 есть замечания; нечитаемый журнал делает вердикт неизвестным' }; },
      target: function (ctx) { return this.today(ctx); }
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
          return { chain: [P.client], config: ['infobase.web.url', 'tools.enterprise.additional-launch-keys (необязательно)'],
                   note: 'адрес один — клиентский, поэтому тонкий клиент идёт по нему без ключа; Конфигуратор, толстый и обычный отказывают. infobase.user и infobase.password здесь принадлежат шлюзу и клиенту не передаются' };
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
      applies: function (ctx) { return ctx.tools.edt ? null : { kind: 'tool', why: 'нет EDT CLI', fix: 'поставьте EDT' }; },
      today: function (ctx) { return { chain: [P.edt], config: ['format', 'source-set[]', 'tools.edt_cli.path'], note: 'только CLI, в MCP не публикуется' }; },
      target: function (ctx) { return this.today(ctx); }
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

  // Порядок первого запуска зависит от предмета: у внешних обработок нет базы,
  // поэтому и шагов с базой в их порядке быть не должно.
  function FIRST_RUN(ctx) {
    if (ctx.type === 'EXTERNAL') return ['config-init', 'make', 'convert'];
    return ['config-init', 'init', 'build', 'test', 'dump'];
  }

  return { AXES: AXES, SCENARIOS: SCENARIOS, PROVIDERS: P, compute: compute, FIRST_RUN: FIRST_RUN };
})();
