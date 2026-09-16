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
