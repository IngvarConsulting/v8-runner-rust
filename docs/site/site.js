// Общая оболочка справки. Исходное HTML-меню работает и без JavaScript.
(function () {
  var bar = document.querySelector('.topbar .in');
  var oldNav = bar && bar.querySelector('nav');
  var page = document.querySelector('main.page');
  if (!oldNav || !page) return;
  function node(tag, cls, text) {
    var el = document.createElement(tag);
    if (cls) el.className = cls;
    if (text) el.textContent = text;
    return el;
  }
  var sidebar = node('nav', 'reference-nav');
  sidebar.id = 'reference-navigation';
  sidebar.setAttribute('aria-label', 'Разделы справки');
  // Названия и адреса принадлежат HTML-меню; здесь только распределение по группам.
  var groups = [
    ['Начало', ['index.html', 'glossary.html']],
    ['Проект', ['project.html']],
    ['CLI', []],
    ['Модель работы', ['platform.html', 'problems.html', 'sources.html']],
    ['Сценарии', ['scenarios.html', 'usecases.html']],
    ['Устройство', ['architecture.html', 'deployments.html']]
  ];
  var links = Array.prototype.slice.call(oldNav.querySelectorAll('a:not(.gh)'));
  groups.forEach(function (group) {
    var section = node('details', 'reference-group');
    section.open = true;
    section.appendChild(node('summary', '', group[0]));
    var subgroups = {};
    links.filter(function (link) { return group[1].indexOf(link.getAttribute('href')) >= 0 || (group[0] === 'CLI' && link.classList.contains('cli-operation')) || (group[0] === 'Проект' && link.classList.contains('project-variant')); })
      .forEach(function (link) {
        var copy = link.cloneNode(true);
        if (link.classList.contains('cli-operation') || link.classList.contains('project-variant')) {
          var label = link.classList.contains('project-variant') ? 'Варианты' : link.dataset.cliGroup;
          if (!subgroups[label]) {
            var nested = node('details', 'reference-cli-group');
            nested.appendChild(node('summary', '', label));
            section.appendChild(nested); subgroups[label] = nested;
          }
          subgroups[label].appendChild(copy);
        } else section.appendChild(copy);
      });
    sidebar.appendChild(section);
  });
  links.filter(function (link) {
    return !link.classList.contains('cli-operation') && !link.classList.contains('project-variant') && !groups.some(function (group) { return group[1].indexOf(link.getAttribute('href')) >= 0; });
  }).forEach(function (link) { sidebar.appendChild(link.cloneNode(true)); });
  function markOperation() {
    sidebar.querySelectorAll('.cli-operation, .project-variant').forEach(function (link) {
      var active = link.getAttribute('href').split('#')[0] === location.pathname.split('/').pop();
      if (active) { link.setAttribute('aria-current', 'location'); link.parentElement.open = true; }
      else link.removeAttribute('aria-current');
    });
  }
  markOperation(); window.addEventListener('hashchange', markOperation);
  var toc = node('div', 'reference-toc');
  toc.appendChild(node('div', 'reference-caption', 'На этой странице'));
  Array.prototype.forEach.call(page.querySelectorAll('h2'), function (heading, index) {
    if (!heading.id) {
      var id = 'section-' + (index + 1);
      while (document.getElementById(id)) id += '-';
      heading.id = id;
    }
    var link = node('a', '', heading.textContent.trim());
    link.href = '#' + heading.id;
    toc.appendChild(link);
  });
  if (toc.children.length > 1) sidebar.appendChild(toc);
  var brand = node('a', 'reference-brand');
  brand.href = 'https://ingvar.pro/';
  brand.title = 'Ingvar Consulting';
  brand.setAttribute('aria-label', 'Ingvar Consulting');
  var brandLight = node('img', 'reference-brand-light');
  brandLight.src = 'assets/ingvar-consulting-dark.svg';
  brandLight.alt = '';
  var brandDark = node('img', 'reference-brand-dark');
  brandDark.src = 'assets/ingvar-consulting-light.svg';
  brandDark.alt = '';
  [brandLight, brandDark].forEach(function (image) { image.width = 886; image.height = 156; brand.appendChild(image); });
  sidebar.appendChild(brand);
  var toggle = node('button', 'reference-button reference-menu', 'Разделы');
  toggle.type = 'button';
  toggle.setAttribute('aria-controls', sidebar.id);
  toggle.setAttribute('aria-expanded', 'false');
  var appearance = node('div', 'reference-appearance');
  var theme = node('button', 'reference-button reference-theme');
  theme.type = 'button';
  theme.title = 'Оформление справки';
  theme.setAttribute('aria-label', 'Оформление справки');
  theme.setAttribute('aria-haspopup', 'dialog');
  theme.setAttribute('aria-controls', 'reference-look');
  theme.setAttribute('aria-expanded', 'false');
  theme.setAttribute('aria-pressed', 'false');
  // Тот же значок оформления, что в справке bsl.js.
  theme.innerHTML = '<svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true"><circle cx="7" cy="7" r="5.6" fill="none" stroke="currentColor" stroke-width="1.2"/><path d="M7 1.4a5.6 5.6 0 0 1 0 11.2z" fill="currentColor"/></svg>';
  var look = node('div', 'reference-look');
  look.id = 'reference-look';
  look.hidden = true;
  look.setAttribute('role', 'dialog');
  look.setAttribute('aria-label', 'Оформление справки');
  var legend = node('legend', 'reference-look-heading', 'Тема справки');
  var choices = node('fieldset', 'reference-theme-choices');
  choices.appendChild(legend);
  var segments = node('div', 'reference-segments');
  var modes = ['auto', 'light', 'dark'];
  var names = ['Как в системе', 'Светлая', 'Тёмная'];
  var radios = [];
  // Палитры справки bsl.js: client/app/dev-theme.ts, CODE_THEMES.
  var codeThemes = [
    {"id":"vscode","title":"VS Code","names":{"light":"Light Modern","dark":"Dark Modern"},"light":{"bg":"#ffffff","fg":"#3b3b3b","key":"#0451a5","string":"#a31515","number":"#098658","boolean":"#0000ff","null":"#0000ff","date":"#811f3f","keyword":"#0000ff","primitive":"#267f99","type":"#267f99","name":"#795e26","comment":"#008000","line":"#6e7681","selection":"#add6ff"},"dark":{"bg":"#1f1f1f","fg":"#cccccc","key":"#9cdcfe","string":"#ce9178","number":"#b5cea8","boolean":"#569cd6","null":"#569cd6","date":"#d16969","keyword":"#569cd6","primitive":"#4ec9b0","type":"#4ec9b0","name":"#dcdcaa","comment":"#6a9955","line":"#6e7681","selection":"#264f78"}},
    {"id":"intellij","title":"IntelliJ","names":{"light":"IntelliJ Light","dark":"Darcula"},"light":{"bg":"#ffffff","fg":"#080808","key":"#871094","string":"#067d17","number":"#1750eb","boolean":"#0033b3","null":"#0033b3","date":"#9e880d","keyword":"#0033b3","primitive":"#0033b3","type":"#000000","name":"#00627a","comment":"#8c8c8c","line":"#adadad","selection":"#a6d2ff"},"dark":{"bg":"#2b2b2b","fg":"#a9b7c6","key":"#9876aa","string":"#6a8759","number":"#6897bb","boolean":"#cc7832","null":"#cc7832","date":"#bbb529","keyword":"#cc7832","primitive":"#cc7832","type":"#a9b7c6","name":"#ffc66d","comment":"#808080","line":"#606366","selection":"#214283"}},
    {"id":"github","title":"GitHub","names":{"light":"Light","dark":"Dark"},"light":{"bg":"#ffffff","fg":"#1f2328","key":"#0550ae","string":"#0a3069","number":"#0550ae","boolean":"#0550ae","null":"#0550ae","date":"#116329","keyword":"#cf222e","primitive":"#0550ae","type":"#953800","name":"#8250df","comment":"#6e7781","line":"#8c959f","selection":"#b6e3ff"},"dark":{"bg":"#0d1117","fg":"#e6edf3","key":"#79c0ff","string":"#a5d6ff","number":"#79c0ff","boolean":"#79c0ff","null":"#79c0ff","date":"#7ee787","keyword":"#ff7b72","primitive":"#79c0ff","type":"#ffa657","name":"#d2a8ff","comment":"#8b949e","line":"#6e7681","selection":"#264f78"}},
    {"id":"solarized","title":"Solarized","names":{"light":"Light","dark":"Dark"},"light":{"bg":"#fdf6e3","fg":"#586e75","key":"#268bd2","string":"#299d94","number":"#d33682","boolean":"#cb4b16","null":"#cb4b16","date":"#6c71c4","keyword":"#829600","primitive":"#b28700","type":"#b28700","name":"#268bd2","comment":"#93a1a1","line":"#93a1a1","selection":"#eee8d5"},"dark":{"bg":"#002b36","fg":"#93a1a1","key":"#268bd2","string":"#2aa198","number":"#d33682","boolean":"#cb4b16","null":"#cb4b16","date":"#6c71c4","keyword":"#859900","primitive":"#b58900","type":"#b58900","name":"#268bd2","comment":"#657b83","line":"#586e75","selection":"#073642"}},
    {"id":"gruvbox","title":"Gruvbox","names":{"light":"Light","dark":"Dark"},"light":{"bg":"#fbf1c7","fg":"#3c3836","key":"#076678","string":"#79740e","number":"#8f3f71","boolean":"#8f3f71","null":"#8f3f71","date":"#af3a03","keyword":"#9d0006","primitive":"#b57614","type":"#b57614","name":"#427b58","comment":"#928374","line":"#a89984","selection":"#ebdbb2"},"dark":{"bg":"#282828","fg":"#ebdbb2","key":"#83a598","string":"#b8bb26","number":"#d3869b","boolean":"#d3869b","null":"#d3869b","date":"#fe8019","keyword":"#fb4934","primitive":"#fabd2f","type":"#fabd2f","name":"#8ec07c","comment":"#928374","line":"#7c6f64","selection":"#504945"}},
    {"id":"one","title":"One","names":{"light":"One Light","dark":"One Dark"},"light":{"bg":"#fafafa","fg":"#383a42","key":"#e45649","string":"#50a14f","number":"#986801","boolean":"#986801","null":"#986801","date":"#0184bc","keyword":"#a626a4","primitive":"#c18401","type":"#c18401","name":"#4078f2","comment":"#a0a1a7","line":"#9d9d9f","selection":"#e5e5e6"},"dark":{"bg":"#282c34","fg":"#abb2bf","key":"#e06c75","string":"#98c379","number":"#d19a66","boolean":"#d19a66","null":"#d19a66","date":"#56b6c2","keyword":"#c678dd","primitive":"#e5c07b","type":"#e5c07b","name":"#61afef","comment":"#7f848e","line":"#636d83","selection":"#3e4451"}}
  ];
  var codeId = 'vscode';
  var codeOptions = [];
  var systemTheme = window.matchMedia('(prefers-color-scheme: dark)');
  var mode = 'auto';
  function readCodeTheme() {
    try {
      var saved = localStorage.getItem('v8-runner:code-theme');
      codeId = codeThemes.some(function (item) { return item.id === saved; }) ? saved : 'vscode';
    } catch (error) {}
  }
  function applyCodeTheme() {
    var dark = mode === 'dark' || (mode === 'auto' && systemTheme.matches);
    var selected = codeThemes.filter(function (item) { return item.id === codeId; })[0];
    var colors = selected[dark ? 'dark' : 'light'];
    Object.keys(colors).forEach(function (role) {
      document.documentElement.style.setProperty('--code-' + role, colors[role]);
    });
    codeOptions.forEach(function (option) {
      option.radio.checked = option.item.id === codeId;
      option.variant.textContent = option.item.names[dark ? 'dark' : 'light'];
      var palette = option.item[dark ? 'dark' : 'light'];
      option.swatch.style.backgroundColor = palette.bg;
      option.dots.forEach(function (dot, index) {
        dot.style.backgroundColor = palette[['key', 'string', 'number', 'keyword', 'type'][index]];
      });
    });
  }
  // Декоративная раскраска команд и YAML; строки разбираются раньше комментариев.
  // Исходный текст не меняется, неизвестные конструкции остаются обычным текстом.
  function paintCode(target, text) {
    var fragment = document.createDocumentFragment();
    var tokens = /(https?:\/\/[^\s"\']+|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|#[^\n]*|(?:^|\n)[ \t]*[A-Za-z_][\w-]*(?=:)|--[\w-]+|\b\d+(?:\.\d+)?\b|\b(?:true|false|null|v8-runner|init|push|pull|test|check|clone|make|infobase|extensions|create|all)\b)/g;
    var offset = 0;
    var match;
    while ((match = tokens.exec(text))) {
      fragment.appendChild(document.createTextNode(text.slice(offset, match.index)));
      var value = match[0];
      var kind = /^https?:/.test(value) ? 'url' : /^["']/.test(value) ? 'string' : value.charAt(0) === '#' ? 'comment'
        : /^--/.test(value) ? 'keyword' : /^\d/.test(value) ? 'number'
        : /^(true|false)$/.test(value) ? 'boolean' : value === 'null' ? 'null'
        : text.charAt(tokens.lastIndex) === ':' ? 'key' : 'name';
      var span = node('span', 'reference-code-' + kind, value);
      fragment.appendChild(span);
      offset = tokens.lastIndex;
    }
    fragment.appendChild(document.createTextNode(text.slice(offset)));
    target.replaceChildren(fragment);
  }
  Array.prototype.forEach.call(page.querySelectorAll('pre > code'), function (block) {
    // Не разрушать ссылки или иную авторскую разметку в блоке.
    if (block.children.length) return;
    block.parentElement.classList.add('reference-code-block');
    var observer = new MutationObserver(repaint);
    function repaint() {
      var text = block.textContent;
      observer.disconnect();
      paintCode(block, text);
      observer.observe(block, { childList: true, subtree: true, characterData: true });
    }
    repaint();
    // «Подбор» обновляет YAML через textContent; раскраска следует за его владельцем.
  });

  function readTheme() {
    try {
      var saved = localStorage.getItem('v8-runner:theme');
      mode = modes.indexOf(saved) >= 0 ? saved : 'auto';
    } catch (error) {}
  }
  function applyTheme() {
    if (mode === 'auto') document.documentElement.removeAttribute('data-theme');
    else document.documentElement.setAttribute('data-theme', mode);
    radios.forEach(function (radio) { radio.checked = radio.value === mode; });
    applyCodeTheme();
  }
  modes.forEach(function (value, index) {
    var option = node('label', 'reference-theme-option');
    var radio = node('input');
    radio.type = 'radio';
    radio.name = 'reference-theme';
    radio.value = value;
    radio.addEventListener('change', function () {
      if (!radio.checked) return;
      mode = value;
      applyTheme();
      try { localStorage.setItem('v8-runner:theme', mode); } catch (error) {}
    });
    option.appendChild(radio);
    option.appendChild(node('span', '', names[index]));
    segments.appendChild(option);
    radios.push(radio);
  });
  choices.appendChild(segments);
  look.appendChild(choices);
  var codeChoices = node('fieldset', 'reference-theme-choices reference-code-choices');
  codeChoices.appendChild(node('legend', 'reference-look-heading', 'Раскраска кода'));
  codeThemes.forEach(function (item) {
    var option = node('label', 'reference-code-option');
    var radio = node('input');
    radio.type = 'radio';
    radio.name = 'reference-code-theme';
    radio.value = item.id;
    radio.addEventListener('change', function () {
      if (!radio.checked) return;
      codeId = item.id;
      applyCodeTheme();
      try { localStorage.setItem('v8-runner:code-theme', codeId); } catch (error) {}
    });
    var row = node('span', 'reference-code-row');
    row.appendChild(node('span', '', item.title));
    var variant = node('small');
    row.appendChild(variant);
    var swatch = node('span', 'reference-code-swatch');
    swatch.setAttribute('aria-hidden', 'true');
    var dots = [];
    for (var i = 0; i < 5; i += 1) {
      var dot = node('i');
      swatch.appendChild(dot);
      dots.push(dot);
    }
    row.appendChild(swatch);
    option.appendChild(radio);
    option.appendChild(row);
    codeChoices.appendChild(option);
    codeOptions.push({ item: item, radio: radio, variant: variant, swatch: swatch, dots: dots });
  });
  look.appendChild(codeChoices);
  var sample = node('pre', 'reference-code-block reference-code-sample');
  sample.setAttribute('aria-label', 'Образец раскраски');
  paintCode(sample, '# Проверить проект\nv8-runner test yaxunit all\nsourcePath: "src"\npartial: true\ntimeout: 300000');
  look.appendChild(sample);

  appearance.appendChild(theme);
  appearance.appendChild(look);
  function closeLook(returnFocus) {
    look.hidden = true;
    theme.setAttribute('aria-expanded', 'false');
    theme.setAttribute('aria-pressed', 'false');
    if (returnFocus) theme.focus();
  }
  theme.addEventListener('click', function () {
    closeSearch();
    if (!look.hidden) { closeLook(false); return; }
    look.hidden = false;
    theme.setAttribute('aria-expanded', 'true');
    theme.setAttribute('aria-pressed', 'true');
    radios[modes.indexOf(mode)].focus();
  });
  document.addEventListener('mousedown', function (event) {
    if (!look.hidden && !appearance.contains(event.target)) closeLook(false);
  });
  window.addEventListener('storage', function (event) {
    if (event.key === 'v8-runner:theme' || event.key === 'v8-runner:code-theme' || event.key === null) {
      readTheme(); readCodeTheme(); applyTheme();
    }
  });
  readTheme();
  readCodeTheme();
  applyTheme();
  systemTheme.addEventListener('change', function () { if (mode === 'auto') applyCodeTheme(); });
  var narrowLayout = window.matchMedia('(max-width: 1000px)');
  var desktopCollapsed = false;
  try { desktopCollapsed = localStorage.getItem('v8-runner:navigation-collapsed') === 'true'; } catch (error) {}
  // Стрелка и размер кнопки как у боковой панели редактора моков bsl.js (dev-tools.ts).
  function menuIcon(button, open) {
    button.innerHTML = '<svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="' + (open ? 'M10 4l-4 4 4 4' : 'M6 4l4 4-4 4') + '"/></svg>';
    button.title = open ? 'Свернуть меню' : 'Развернуть меню';
    button.setAttribute('aria-label', button.title);
  }
  function saveCollapsed() {
    try { localStorage.setItem('v8-runner:navigation-collapsed', String(desktopCollapsed)); } catch (error) {}
  }
  function syncMenu() {
    var open = narrowLayout.matches ? document.body.classList.contains('reference-menu-open') : !desktopCollapsed;
    document.body.classList.toggle('reference-collapsed', !narrowLayout.matches && desktopCollapsed);
    toggle.setAttribute('aria-expanded', String(open));
    menuIcon(toggle, open);
  }
  function closeMenu(returnFocus) {
    if (narrowLayout.matches) document.body.classList.remove('reference-menu-open');
    else { desktopCollapsed = true; saveCollapsed(); }
    syncMenu();
    if (returnFocus) toggle.focus();
  }
  function openMenu() {
    if (narrowLayout.matches) document.body.classList.add('reference-menu-open');
    else { desktopCollapsed = false; saveCollapsed(); }
    syncMenu();
  }
  toggle.addEventListener('click', function () {
    if (toggle.getAttribute('aria-expanded') === 'true') closeMenu(false);
    else { openMenu(); sidebar.querySelector('summary').focus(); }
  });
  narrowLayout.addEventListener('change', function () {
    endDrag();
    document.body.classList.remove('reference-menu-open');
    if (sidebar.contains(document.activeElement) || document.activeElement === divider) toggle.focus();
    syncMenu();
  });
  syncMenu();
  sidebar.addEventListener('click', function (event) {
    var link = event.target.closest('a');
    if (!link) return;
    if (narrowLayout.matches) closeMenu(false);
    // После локального перехода фокус не остаётся в скрытой мобильной панели.
    if (link.getAttribute('href').charAt(0) === '#') {
      var heading = document.getElementById(link.hash.slice(1));
      if (heading) { heading.tabIndex = -1; heading.focus({ preventScroll: true }); }
    }
  });
  var searchWrap = node('div', 'reference-site-search');
  var search = node('input');
  search.type = 'search';
  search.placeholder = 'Поиск по сайту  /';
  search.setAttribute('aria-label', 'Поиск по сайту');
  search.setAttribute('aria-controls', 'reference-search-results');
  search.setAttribute('aria-expanded', 'false');
  var results = node('div', 'reference-search-results');
  results.id = 'reference-search-results';
  results.hidden = true;
  results.setAttribute('role', 'region');
  results.setAttribute('aria-label', 'Результаты поиска');
  searchWrap.appendChild(search);
  searchWrap.appendChild(results);
  var indexPromise = null;
  function normalize(text) { return text.toLocaleLowerCase('ru').replace(/ё/g, 'е'); }
  function loadIndex() {
    if (!indexPromise) {
      indexPromise = fetch('search-index.json?v=1')
        .then(function (response) { if (!response.ok) throw new Error('index'); return response.json(); })
        .then(function (index) {
          if (!Array.isArray(index) || !index.every(function (entry) {
            return typeof entry.url === 'string' && /^[a-z-]+\.html(?:\?selection=[%\w.-]+)?(?:#[\w-]+)?$/.test(entry.url)
              && typeof entry.page === 'string' && typeof entry.section === 'string' && typeof entry.text === 'string';
          })) throw new Error('index');
          return index.map(function (entry) {
            return { entry: entry, normalized: normalize(entry.page + ' ' + entry.section + ' ' + entry.text) };
          });
        }).catch(function (error) { indexPromise = null; throw error; });
    }
    return indexPromise;
  }
  function closeSearch() { results.hidden = true; search.setAttribute('aria-expanded', 'false'); }
  function message(text) {
    results.replaceChildren(node('p', 'reference-search-status', text));
    results.firstChild.setAttribute('role', 'status');
  }
  function showSearch() {
    var query = search.value.trim();
    closeLook(false);
    if (!query) { closeSearch(); return; }
    results.hidden = false;
    search.setAttribute('aria-expanded', 'true');
    message('Ищем…');
    loadIndex().then(function (index) {
      if (results.hidden || search.value.trim() !== query) return;
      var words = normalize(query).split(/\s+/);
      var seen = {};
      var matches = index.filter(function (item) { return words.every(function (word) { return item.normalized.indexOf(word) >= 0; }); })
        .sort(function (a, b) {
          function score(item) { return words.reduce(function (sum, word) { return sum + (normalize(item.entry.section).indexOf(word) >= 0 ? 1 : 0); }, 0); }
          return score(b) - score(a);
        }).filter(function (item) { var key = item.entry.url.split('?')[0].split('#')[0] + '#' + (item.entry.url.split('#')[1] || ''); if (seen[key]) return false; seen[key] = true; return true; });
      if (!matches.length) { message('Ничего не найдено. Попробуйте другие слова.'); return; }
      message('Найдено: ' + matches.length + (matches.length > 40 ? '. Показаны первые 40 — уточните запрос.' : ''));
      matches.slice(0, 40).forEach(function (item) {
        var entry = item.entry;
        var result = node('a', 'reference-search-result');
        result.href = entry.url;
        result.appendChild(node('strong', '', entry.page + (entry.section !== entry.page ? ' · ' + entry.section : '')));
        var at = normalize(entry.text).indexOf(words[0]);
        var start = Math.max(0, at - 70);
        var excerpt = (start ? '…' : '') + entry.text.slice(start, start + 220) + (entry.text.length > start + 220 ? '…' : '');
        result.appendChild(node('span', '', excerpt));
        results.appendChild(result);
      });
    }).catch(function () {
      if (results.hidden || search.value.trim() !== query) return;
      message('Не удалось загрузить поиск.');
      var retry = node('button', 'reference-button', 'Повторить');
      retry.type = 'button';
      retry.addEventListener('click', showSearch);
      results.appendChild(retry);
    });
  }
  search.addEventListener('input', showSearch);
  search.addEventListener('focus', function () { if (search.value.trim()) showSearch(); });
  searchWrap.addEventListener('keydown', function (event) {
    if (event.key === 'Escape' && !results.hidden) { event.stopPropagation(); search.focus(); closeSearch(); }
    if ((event.key === 'Enter' || event.key === 'ArrowDown') && event.target === search) {
      var first = results.querySelector('a');
      if (first && !results.hidden) { event.preventDefault(); if (event.key === 'Enter') first.click(); else first.focus(); }
    }
  });
  document.addEventListener('mousedown', function (event) { if (!searchWrap.contains(event.target)) closeSearch(); });
  document.addEventListener('keydown', function (event) {
    var editing = event.target.closest('input, textarea, select, [contenteditable="true"]');
    if (event.key === '/' && !editing && !event.ctrlKey && !event.metaKey && !event.altKey) {
      event.preventDefault();
      search.focus();
    }
    if (event.key === 'Escape') {
      if (!look.hidden) closeLook(true);
      else if (document.body.classList.contains('reference-menu-open')) closeMenu(true);
    }

  });
  bar.insertBefore(toggle, bar.firstChild);
  bar.appendChild(node('span', 'reference-title', 'справка'));
  bar.appendChild(searchWrap);
  bar.appendChild(appearance);
  var github = oldNav.querySelector('.gh');
  if (github) bar.appendChild(github);
  document.body.insertBefore(sidebar, page);
  // Граница навигации: мышь, клавиатура и сохранение выбранной ширины.
  var divider = node('div', 'reference-divider');
  divider.tabIndex = 0;
  divider.setAttribute('role', 'separator');
  divider.setAttribute('aria-orientation', 'vertical');
  divider.setAttribute('aria-label', 'Ширина навигации');
  divider.setAttribute('aria-valuemin', '180');
  divider.title = 'Потяните для изменения ширины. Двойной щелчок — сбросить.';
  var preferredWidth = 270;
  var navigationWidth = 270;
  var drag = null;
  try {
    var storedWidth = Number(localStorage.getItem('v8-runner:navigation-width'));
    if (Number.isFinite(storedWidth) && storedWidth >= 180 && storedWidth <= 600) preferredWidth = storedWidth;
  } catch (error) {}
  function fitNavigation() {
    var maximum = Math.max(180, Math.min(600, window.innerWidth - 320));
    navigationWidth = Math.max(180, Math.min(maximum, preferredWidth));
    document.documentElement.style.setProperty('--navigation-width', navigationWidth + 'px');
    divider.setAttribute('aria-valuemax', String(maximum));
    divider.setAttribute('aria-valuenow', String(navigationWidth));
  }
  function changeWidth(value) {
    preferredWidth = Math.max(180, Math.min(600, window.innerWidth - 320, value));
    fitNavigation();
  }
  function saveWidth() {
    try { localStorage.setItem('v8-runner:navigation-width', String(preferredWidth)); } catch (error) {}
  }
  function endDrag() {
    if (!drag) return;
    var pointerId = drag.pointerId;
    drag = null;
    document.body.classList.remove('reference-resizing');
    if (divider.hasPointerCapture(pointerId)) divider.releasePointerCapture(pointerId);
    saveWidth();
  }
  divider.addEventListener('pointerdown', function (event) {
    if (event.button !== 0 || drag) return;
    event.preventDefault();
    divider.focus();
    drag = { x: event.clientX, width: navigationWidth, pointerId: event.pointerId };
    divider.setPointerCapture(event.pointerId);
    document.body.classList.add('reference-resizing');
  });
  divider.addEventListener('pointermove', function (event) {
    if (drag && drag.pointerId === event.pointerId) changeWidth(drag.width + event.clientX - drag.x);
  });
  ['pointerup', 'pointercancel', 'lostpointercapture'].forEach(function (type) {
    divider.addEventListener(type, endDrag);
  });
  divider.addEventListener('dblclick', function () { changeWidth(270); saveWidth(); });
  divider.addEventListener('keydown', function (event) {
    if (['ArrowLeft', 'ArrowRight', 'Home', 'End'].indexOf(event.key) < 0) return;
    event.preventDefault();
    changeWidth(event.key === 'Home' ? 180 : event.key === 'End' ? 600 : navigationWidth + (event.key === 'ArrowLeft' ? -16 : 16));
    saveWidth();
  });
  window.addEventListener('resize', function () { endDrag(); fitNavigation(); });
  document.body.insertBefore(divider, page);
  fitNavigation();
  oldNav.hidden = true;
  // Включаем оформление только после успешного построения навигации.
  document.body.classList.add('reference-shell');
})();

// Счётчик звёзд у ссылки на GitHub. Значение живёт час в localStorage, чтобы на каждой
// странице не ходить в API; без сети и в приватном режиме остаётся одна иконка.
(function () {
  var REPO = 'IngvarConsulting/v8-runner-rust';
  var KEY = 'v8-runner:stars';
  var TTL_MS = 3600000;
  var slot = document.querySelector('.gh-stars');
  if (!slot) return;

  function show(count) {
    if (typeof count !== 'number') return;
    slot.textContent = count.toLocaleString('ru-RU');
    slot.hidden = false;
    var link = slot.closest('a');
    if (link) link.setAttribute('aria-label', 'GitHub, звёзд: ' + count);
  }

  var cached = null;
  try {
    cached = JSON.parse(localStorage.getItem(KEY) || 'null');
  } catch (error) {
    cached = null;
  }
  if (cached && typeof cached.count === 'number' && Date.now() - cached.at < TTL_MS) {
    show(cached.count);
    return;
  }

  fetch('https://api.github.com/repos/' + REPO, { headers: { Accept: 'application/vnd.github+json' } })
    .then(function (response) { return response.ok ? response.json() : null; })
    .then(function (data) {
      if (!data || typeof data.stargazers_count !== 'number') return;
      show(data.stargazers_count);
      try {
        localStorage.setItem(KEY, JSON.stringify({ count: data.stargazers_count, at: Date.now() }));
      } catch (error) {
        // приватный режим: просто без кеша
      }
    })
    .catch(function () {
      // счётчика не будет, ссылка работает
    });
})();

// Широкая таблица на телефоне читается карточками: подписи берём из шапки, первая ячейка
// становится заголовком карточки. Таблицы «ключ — значение» (без шапки) не трогаем, у
// `#result` на странице команд своя разметка с `data-l`.
(function () {
  var tables = document.querySelectorAll('.tablewrap table:not(#result)');
  Array.prototype.forEach.call(tables, function (table) {
    var heads = Array.prototype.map.call(table.querySelectorAll('thead th'), function (th) {
      return th.textContent.trim();
    });
    if (heads.length < 3) return;
    table.classList.add('stack');
    Array.prototype.forEach.call(table.querySelectorAll('tbody tr'), function (row) {
      Array.prototype.forEach.call(row.children, function (cell, index) {
        if (index > 0 && heads[index]) cell.setAttribute('data-label', heads[index]);
      });
    });
  });
})();

// Схемы и оставшиеся широкие таблицы прокручиваются вбок. Прокрутку надо показать: у
// края появляется тень, а при первом появлении на экране блок один раз дёргается.
(function () {
  var reduceMotion = window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  var nudged = typeof WeakSet === 'function' ? new WeakSet() : null;

  function nudge(box) {
    if (reduceMotion || !nudged || nudged.has(box) || box.scrollLeft > 0) return;
    nudged.add(box);
    try {
      box.scrollTo({ left: 28, behavior: 'smooth' });
      window.setTimeout(function () { box.scrollTo({ left: 0, behavior: 'smooth' }); }, 420);
    } catch (error) {
      // старый браузер без плавной прокрутки: тень у края всё равно остаётся подсказкой
    }
  }

  var watcher = typeof IntersectionObserver === 'function'
    ? new IntersectionObserver(function (entries) {
        entries.forEach(function (entry) {
          if (!entry.isIntersecting) return;
          nudge(entry.target);
          watcher.unobserve(entry.target);
        });
      }, { threshold: 0.35 })
    : null;

  function mark() {
    var boxes = document.querySelectorAll('.diagram, .tablewrap, .axes');
    Array.prototype.forEach.call(boxes, function (box) {
      var scrollable = box.scrollWidth - box.clientWidth > 8;
      box.classList.toggle('scrollx', scrollable);
      if (!scrollable) return;
      if (watcher) watcher.observe(box); else nudge(box);
    });
  }

  mark();
  window.addEventListener('load', mark);
  var resizeTimer = null;
  window.addEventListener('resize', function () {
    window.clearTimeout(resizeTimer);
    resizeTimer = window.setTimeout(mark, 150);
  });
})();

(function () {
  var tabs = Array.from(document.querySelectorAll('[data-install-tab]'));
  if (!tabs.length) return;
  var panels = Array.from(document.querySelectorAll('[data-install-panel]'));
  tabs[0].parentElement.setAttribute('role', 'tablist');
  function select(key) {
    tabs.forEach(function (tab) {
      var active = tab.dataset.installTab === key;
      tab.setAttribute('aria-selected', String(active)); tab.tabIndex = active ? 0 : -1;
    });
    panels.forEach(function (panel) { panel.hidden = panel.dataset.installPanel !== key; });
  }
  tabs.forEach(function (tab, i) {
    var key = tab.dataset.installTab;
    tab.id = 'install-tab-' + key; tab.setAttribute('role', 'tab');
    tab.setAttribute('aria-controls', 'install-panel-' + key);
    var panel = panels.find(function (p) { return p.dataset.installPanel === key; });
    panel.id = 'install-panel-' + key; panel.setAttribute('role', 'tabpanel');
    panel.setAttribute('aria-labelledby', tab.id);
    tab.addEventListener('click', function () { select(key); });
    tab.addEventListener('keydown', function (event) {
      var next = event.key === 'ArrowRight' ? (i + 1) % tabs.length : event.key === 'ArrowLeft' ? (i + tabs.length - 1) % tabs.length : event.key === 'Home' ? 0 : event.key === 'End' ? tabs.length - 1 : -1;
      if (next < 0) return;
      event.preventDefault(); select(tabs[next].dataset.installTab); tabs[next].focus();
    });
  });
  var platform = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || navigator.userAgent;
  var detected = /Win/i.test(platform) ? 'windows' : /Mac/i.test(platform) ? 'macos' : 'linux';
  function selectFromHash() {
    var match = location.hash.match(/^#install-(macos|windows|linux)$/);
    select(match ? match[1] : detected);
    if (match) document.getElementById('install-' + match[1]).scrollIntoView({ block: 'start' });
  }
  selectFromHash();
  window.addEventListener('hashchange', selectFromHash);
})();

(function () {
  var control = document.querySelector('[data-output-format]');
  var source = document.getElementById('output-examples');
  if (!control || !source) return;
  var examples = JSON.parse(source.textContent);
  var commands = Array.from(document.querySelectorAll('[data-output-command] > code'));
  var outputs = Array.from(document.querySelectorAll('[data-output-example] > code'));
  var originals = new Map();
  commands.concat(outputs).forEach(function (code) { originals.set(code, code.textContent); });
  function renderOutputFormat() {
    var json = control.querySelector('input:checked').value === 'json';
    commands.forEach(function (code) {
      var original = originals.get(code);
      code.textContent = json && !/(?:^|\s)--json-message(?=\s|$)/.test(original)
        ? original.replace(/\bv8-runner(?=\s)/g, 'v8-runner --json-message') : original;
    });
    outputs.forEach(function (code) {
      code.textContent = json ? JSON.stringify(examples[code.parentElement.dataset.outputExample], null, 2) : originals.get(code);
    });
  }
  control.addEventListener('change', renderOutputFormat);
  renderOutputFormat();
})();

(function () {
  var icon = '<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="8" y="8" width="12" height="13" rx="2"/><path d="M16 8V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h3"/></svg>';
  document.querySelectorAll('pre > code').forEach(function (code) {
    var button = document.createElement('button');
    button.type = 'button'; button.className = 'code-copy';
    button.innerHTML = icon;
    button.setAttribute('aria-label', 'Скопировать код'); button.title = 'Скопировать';
    var status = document.createElement('span');
    status.className = 'code-copy-status'; status.setAttribute('role', 'status');
    code.parentElement.classList.add('code-copy-enabled');
    code.parentElement.appendChild(button); document.body.appendChild(status);
    var timer;
    button.addEventListener('click', async function () {
      try {
        var field = document.createElement('textarea');
        field.value = code.textContent;
        field.style.cssText = 'position:fixed;left:-9999px;top:0';
        document.body.appendChild(field); field.select();
        var copied;
        try { copied = document.execCommand('copy'); }
        finally { field.remove(); button.focus(); }
        if (!copied) await navigator.clipboard.writeText(code.textContent);
        status.textContent = 'Скопировано';

      } catch (error) {
        status.textContent = 'Не удалось скопировать';
      }
      var rect = button.getBoundingClientRect();
      status.style.left = (rect.left + rect.width / 2) + 'px';
      status.style.top = (rect.top - 9) + 'px';
      status.classList.add('visible');
      clearTimeout(timer);
      timer = setTimeout(function () { status.classList.remove('visible'); status.textContent = ''; button.title = 'Скопировать'; }, 2000);
    });
  });
})();
