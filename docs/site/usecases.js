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
  { id: 'start', title: 'Завести проект из готовых исходников',
    who: 'разработчик, у которого есть каталог с выгрузкой и больше ничего',
    pre: 'установлена платформа 1С',
    steps: [
      { id: 'config-init', in: ['repo'], out: ['cfg'], note: 'раннер находит наборы исходников' },
      { id: 'init', in: ['cfg'], out: ['ib'], note: 'появляется пустая база' },
      { id: 'build', in: ['repo', 'cfg'], out: ['ib', 'state'], note: 'исходники попадают в базу' },
      { id: 'test', in: ['ib'], out: ['reports'], note: 'проверяем, что база живая' }
    ] },

  { id: 'daily', title: 'Ежедневный цикл разработки',
    who: 'разработчик, правящий исходники в редакторе',
    pre: 'проект заведён',
    steps: [
      { id: 'build', in: ['repo', 'state'], out: ['ib', 'state', 'genid'], note: 'грузится только изменённое; после загрузки запоминается новый идентификатор поколения' },
      { id: 'syntax', in: ['ib'], out: ['issues'], note: 'проверка синтаксиса' },
      { id: 'test', in: ['ib'], out: ['reports'], note: 'прогон тестов' }
    ] },

  { id: 'reverse', title: 'Правки сделали в Конфигураторе',
    who: 'разработчик, который правил базу руками',
    pre: 'рабочее дерево чистое: незакоммиченных правок в исходниках нет',
    warn: 'Раннер не сливает исходники. Полная выгрузка заменяет каталог набора целиком — старое уходит в backup на время публикации, но незакоммиченные правки в дереве будут потеряны. Слияние с чужими изменениями делает система контроля версий, поэтому выгружать нужно в чистое дерево.',
    steps: [
      { id: 'dump', in: ['ib', 'genid'], out: ['repo', 'state', 'genid'], note: 'сначала спрашивает идентификатор поколения: если он тот же, что при прошлой синхронизации, в базе ничего не меняли и выгружать нечего' },
      { manual: true, title: 'слияние', in: ['repo'], out: ['vcs'], note: 'коммит, затем merge или rebase с чужими правками — вне раннера' },
      { id: 'build', in: ['vcs'], out: ['ib', 'state'], note: 'объединённое состояние возвращается в базу' },
      { id: 'syntax', in: ['ib'], out: ['issues'], note: 'проверка после слияния' }
    ] },

  { id: 'release', title: 'Собрать поставку',
    who: 'релиз-инженер',
    pre: 'сборка прошла',
    steps: [
      { id: 'build', in: ['repo'], out: ['ib'], note: 'база приводится к состоянию исходников' },
      { id: 'make', in: ['ib', 'repo'], out: ['cf', 'epf'], note: 'из базы — .cf и .cfe, из внешних наборов — .epf и .erf' },
      { id: 'ib-export', in: ['ib'], out: ['cf'], note: 'если нужна именно конфигурация базы: рабочая или та, что в БД' }
    ] },

  { id: 'clean', title: 'Проверка на чистой базе',
    who: 'CI',
    pre: 'есть эталонный .dt',
    steps: [
      { id: 'ib-dump', cmd: 'v8-runner infobase restore --input ib.dt --replace', in: ['dt'], out: ['ib'], note: 'эталон разворачивается в базу' },
      { id: 'build', in: ['repo'], out: ['ib', 'state'], note: 'накатываем исходники' },
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
    pre: 'есть .cfe или его исходники',
    steps: [
      { id: 'load', in: ['cf'], out: ['ib'], note: 'проба совместимости решает, можно ли ставить' },
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
    pre: 'сервер поднят, объявлен SSH-шлюз',
    steps: [
      { id: 'build', in: ['repo'], out: ['ib', 'state'], note: 'загрузка через шлюз, одной сессией' },
      { id: 'dump', in: ['ib'], out: ['repo'], note: 'выгрузка тем же каналом' },
      { id: 'launch-web', in: ['pub'], out: ['browser'], note: 'адрес даёт сам сервер' }
    ] },

  { id: 'client', title: 'Запустить клиент руками',
    who: 'разработчик, которому нужен Конфигуратор или тонкий клиент',
    pre: 'база есть',
    steps: [
      { id: 'launch', in: ['ib'], out: ['client'], note: 'раннер собирает строку запуска' }
    ] }
];
