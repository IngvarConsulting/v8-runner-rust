// Сценарии: у каждого шага назван вход и выход, поэтому поток данных рисуется из данных,
// а не подписывается руками. id шага ссылается на команду из data.js — так считается покрытие.
window.RUNNER_ARTEFACTS = {
  repo:    { label: 'исходники в репозитории', kind: 'data' },
  other:   { label: 'исходники в другом формате', kind: 'data' },
  cfg:     { label: 'v8project.yaml', kind: 'data' },
  ib:      { label: 'информационная база', kind: 'store' },
  dt:      { label: 'файл .dt', kind: 'data' },
  cf:      { label: '.cf или .cfe', kind: 'data' },
  epf:     { label: '.epf и .erf', kind: 'data' },
  reports: { label: 'отчёты тестов', kind: 'data' },
  issues:  { label: 'замечания', kind: 'data' },
  extlist: { label: 'состав расширений', kind: 'data' },
  state:   { label: 'состояние изменений в workPath', kind: 'store' },
  genid:   { label: 'идентификатор поколения', kind: 'data' },
  pub:     { label: 'публикация на веб-сервере', kind: 'store' },
  browser: { label: 'браузер', kind: 'ext' },
  client:  { label: 'клиент 1С', kind: 'ext' },
  vcs:     { label: 'система контроля версий', kind: 'ext' }
};

window.RUNNER_USECASES = [
  { id: 'adopt', title: 'Завести проект из существующей базы',
    who: 'разработчик, у которого есть база и нет исходников',
    pre: 'установлена платформа 1С, каталог пуст',
    steps: [
      { id: 'clone', in: ['ib'], out: ['repo', 'cfg', 'state', 'genid'], note: 'привязка каталога к базе и первая выгрузка' },
      { id: 'status', in: ['state'], out: [], note: 'пара совпадает; каталог можно зафиксировать в системе контроля версий' }
    ] },

  { id: 'start', title: 'Завести проект из готовых исходников',
    who: 'разработчик, у которого есть каталог с выгрузкой и больше ничего',
    pre: 'установлена платформа 1С',
    steps: [
      { id: 'init', in: ['repo'], out: ['cfg'], note: 'раннер находит наборы исходников' },
      { id: 'infobase-create', in: ['cfg'], out: ['ib'], note: 'появляется пустая база' },
      { id: 'push', in: ['repo', 'cfg'], out: ['ib', 'state'], note: 'исходники попадают в базу' },
      { id: 'test', in: ['ib'], out: ['reports'], note: 'проверяем, что база живая' }
    ] },

  { id: 'daily', title: 'Ежедневный цикл разработки',
    who: 'разработчик, правящий исходники в редакторе',
    pre: 'проект заведён',
    steps: [
      { id: 'push', in: ['repo', 'state'], out: ['ib', 'state', 'genid'], note: 'загружается только изменённое; после загрузки запоминается новый идентификатор поколения' },
      { id: 'check', in: ['ib'], out: ['issues'], note: 'проверка конфигурации и модулей' },
      { id: 'test', in: ['ib'], out: ['reports'], note: 'прогон тестов' }
    ] },

  { id: 'reverse', title: 'Правки сделали в Конфигураторе',
    who: 'разработчик, который правил базу руками',
    pre: 'незафиксированные правки зафиксированы: pull откажется терять то, чего система версий не вернёт',
    warn: 'pull сливает пообъектно: выгрузка ложится поверх, лишнее остаётся, а сводит расхождение система контроля версий. Заменить каталог ровно базой умеет только pull --force.',
    steps: [
      { id: 'status', cmd: 'v8-runner status --deep', in: ['state', 'ib'], out: [], note: 'база ушла вперёд, отправка будет отклонена' },
      { id: 'diff', in: ['ib', 'genid'], out: ['issues'], note: 'какие объекты изменились в базе, без выгрузки' },
      { id: 'pull', in: ['ib', 'genid'], out: ['repo', 'state', 'genid'], note: 'сначала спрашивает идентификатор поколения: если он тот же, что при прошлой синхронизации, в базе ничего не меняли и выгружать нечего' },
      { manual: true, title: 'слияние', in: ['repo'], out: ['vcs'], note: 'коммит, затем merge или rebase с чужими правками — вне раннера' },
      { id: 'push', in: ['vcs'], out: ['ib', 'state'], note: 'объединённое состояние возвращается в базу' },
      { id: 'check', in: ['ib'], out: ['issues'], note: 'проверка после слияния' }
    ] },

  { id: 'release', title: 'Собрать поставку',
    who: 'релиз-инженер',
    pre: 'сборка прошла',
    steps: [
      { id: 'push', in: ['repo'], out: ['ib'], note: 'база приводится к состоянию исходников' },
      { id: 'make', in: ['repo'], out: ['cf', 'epf'], note: 'пакеты собираются из исходников, база не нужна' },
      { id: 'download', in: ['ib'], out: ['cf'], note: 'если нужна именно конфигурация базы: основная или та, что в базе данных' }
    ] },

  { id: 'deferred', title: 'Отложить реструктуризацию до окна обслуживания',
    who: 'администратор рабочей базы',
    pre: 'в базе работают пользователи',
    steps: [
      { id: 'push', cmd: 'v8-runner push --no-apply', in: ['repo'], out: ['ib'], note: 'исходники ложатся в основную конфигурацию; база данных не тронута, сеансы работают' },
      { id: 'sessions', cmd: 'v8-runner sessions deny --message "Обновление" --jobs', in: ['ib'], out: ['ib'], note: 'новые сеансы и регламентные задания не начинаются' },
      { id: 'sessions', cmd: 'v8-runner sessions terminate --all', in: ['ib'], out: ['ib'], note: 'работающие сеансы завершаются с сообщением' },
      { id: 'apply', in: ['ib'], out: ['ib'], note: 'монопольный доступ свободен, реструктуризация идёт' },
      { id: 'sessions', cmd: 'v8-runner sessions allow', in: ['ib'], out: ['ib'], note: 'блокировка снята' }
    ] },

  { id: 'vendor', title: 'Установить обновление поставщика',
    who: 'сопровождающий типовую конфигурацию',
    pre: 'есть пакет .cf новой версии',
    steps: [
      { id: 'upload', cmd: 'v8-runner upload new.cf --mode update', in: ['cf'], out: ['ib'], note: 'обновление конфигурации поставщика по правилам поддержки' },
      { id: 'apply', in: ['ib'], out: ['ib'], note: 'реструктуризация' },
      { id: 'pull', in: ['ib'], out: ['repo'], note: 'новое состояние базы попадает в исходники' }
    ] },

  { id: 'clean', title: 'Проверка на чистой базе',
    who: 'CI',
    pre: 'есть эталонный .dt',
    steps: [
      { id: 'ib-dump', cmd: 'v8-runner infobase restore --input ib.dt --replace', in: ['dt'], out: ['ib'], note: 'эталон разворачивается в базу' },
      { id: 'push', in: ['repo'], out: ['ib', 'state'], note: 'отправляем исходники' },
      { id: 'test', in: ['ib'], out: ['reports'], note: 'полный прогон' }
    ] },

  { id: 'transfer', title: 'Перенести базу на другой контур',
    who: 'администратор',
    pre: 'доступ к обеим базам',
    steps: [
      { id: 'ib-dump', cmd: 'v8-runner infobase dump --output ib.dt', in: ['ib'], out: ['dt'], note: 'снимок базы вместе с данными' },
      { id: 'ib-dump', cmd: 'v8-runner infobase restore --input ib.dt --create', in: ['dt'], out: ['ib'], note: 'разворачиваем на целевом контуре' }
    ] },

  { id: 'ext', title: 'Первая установка расширения',
    who: 'разработчик расширения',
    pre: 'есть исходники расширения',
    steps: [
      { id: 'push', cmd: 'v8-runner push my-ext', in: ['repo'], out: ['ib'], note: 'первая отправка заводит расширение сама' },
      { id: 'extensions', in: ['ib'], out: ['extlist'], note: 'смотрим состав и свойства' }
    ] },

  { id: 'web', title: 'Проверить веб-клиент',
    who: 'разработчик или тестировщик',
    pre: 'в окружении есть Apache или IIS',
    steps: [
      { id: 'publish', in: ['ib', 'cfg'], out: ['pub'], note: 'база публикуется на веб-сервере' },
      { id: 'launch-web', in: ['pub'], out: ['browser'], note: 'открываем по адресу публикации' }
    ] },

  { id: 'edt', title: 'Перейти между EDT и XML платформы',
    who: 'команда, меняющая формат репозитория',
    pre: 'установлен EDT',
    steps: [
      { id: 'convert', in: ['repo'], out: ['other'], note: 'база не участвует' }
    ] },

  { id: 'standalone', title: 'Работа с автономным сервером',
    who: 'разработчик контура на ibsrv',
    pre: 'сервер поднят, объявлены строка подключения прямого шлюза и SSH-шлюз',
    steps: [
      { id: 'push', in: ['repo'], out: ['ib', 'state'], note: 'Конфигуратор по прямому шлюзу; без платформы — SSH-шлюз, одной сессией' },
      { id: 'pull', in: ['ib'], out: ['repo'], note: 'выгрузка ложится у раннера' },
      { id: 'launch-web', in: ['ib'], out: ['browser'], note: 'адрес даёт сам сервер' }
    ] },

  { id: 'client', title: 'Запустить клиент руками',
    who: 'разработчик, которому нужен Конфигуратор или тонкий клиент',
    pre: 'база есть',
    steps: [
      { id: 'launch', in: ['ib'], out: ['client'], note: 'раннер собирает строку запуска' }
    ] }
];
