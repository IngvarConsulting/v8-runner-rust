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
