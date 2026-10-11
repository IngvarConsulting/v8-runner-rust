"""Guard the target-only public reference; search indexing owns article extraction/rendering."""
import html
import json
import re
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent
# Bounded regression classes, not a classifier of Russian prose. Genuine target
# refusals, safety requirements, runtime state and the Plan badge remain valid.
PATTERNS = {
    'implementation-version': r'\bтекущ(?:ая|ей|ую|ий|его|ем|ем)\s+(?:сборк\w*|верси\w*|реализаци\w*|cli\b|формат\w*|команд\w*)',
    'implementation-incomplete': r'(?:ещ[её]\s+не\s+(?:реализован\w*|поддержива\w*|закрепл[её]н\w*)|\bпока\s+(?:не\s+(?:поддержива\w*|уме\w*|выда[её]\w*|реализован\w*)|команда\s+по\s+умолчанию))',
    'adapter-gap': r'(?:\bнет\s+адаптер\w*|адаптер\w*\s+нет\b)',
    'contract-gap': r'формы\s+ошибок.{0,100}не\s+закрепл[её]н\w*',
    'proposal-workaround': r'(?:предлагаем\w*\s+(?:ключ|контракт|формат)|планируемая\s+форма|\bдля\s+(?:текущей|нынешней)\s+версии\s+используйте)',
    'historical-source-default': r'по\s+умолчанию[^.;]{0,90}src/configuration\b',
    'internal-payload-caveat': r'failed-payload',
    'implementation-option-values': r'текущие\s+значения\s+--',
    'description-gap': r'для\s+внешних\s+наборов\s+не\s+описан\w*',
}
COMPILED = {name: re.compile(pattern, re.I | re.S) for name, pattern in PATTERNS.items()}


def violations(text):
    normalized = ' '.join(text.split())
    return [name for name, pattern in COMPILED.items() if pattern.search(normalized)]


def json_strings(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for child in value.values():
            yield from json_strings(child)
    elif isinstance(value, list):
        for child in value:
            yield from json_strings(child)


def check_site():
    # Rebuild instead of trusting a stale committed index. Existing index producer
    # renders all 2304 axis/tool combinations and extracts public article content.
    subprocess.run([sys.executable, str(ROOT / 'build_search_index.py')], check=True)
    records = json.loads((ROOT / 'search-index.json').read_text())
    failures = set()
    for record in records:
        for name in violations(record['text']):
            failures.add((record['url'], name))
    # JSON output examples live outside <main>, so the article index excludes them.
    # Scan their strings and source markup too (comments may reappear on regeneration).
    for path in ROOT.glob('*.html'):
        source = path.read_text()
        plain = html.unescape(re.sub(r'<[^>]*>', ' ', source))
        for name in violations(plain):
            failures.add((path.name, name))
        for raw in re.findall(r'<script\b[^>]*type="application/json"[^>]*>(.*?)</script>', source, re.S):
            for message in json_strings(json.loads(raw)):
                for name in violations(message):
                    failures.add((path.name + ':json', name))
    data = (ROOT / 'data.js').read_text()
    if re.search(r'\btoday\s*:|\b(?:s|this)\.today\b|function\s+compute\s*\(\s*ctx\s*,', data):
        failures.add(('data.js', 'alternative-implementation-model'))
    if re.search(r'id=["\']gap["\']|id=["\']coverage["\']', (ROOT / 'usecases.html').read_text()):
        failures.add(('usecases.html', 'internal-coverage-ui'))
    for location, name in sorted(failures):
        print(f'{location}: {name}', file=sys.stderr)
    if failures:
        raise SystemExit(1)
    print(f'Target content: {len(set(r["url"].split("?")[0].split("#")[0] for r in records))} pages, rendered selections and JSON examples checked')


class RegressionTests(unittest.TestCase):
    def test_rejects_historical_clone_workaround(self):
        self.assertTrue(violations('Используйте src/cf. Пока команда по умолчанию выбирает src/configuration, поэтому в примерах указан явный --source-dir src/cf.'))

    def test_rejects_other_implementation_gap_forms(self):
        for text in [
            'Ключ --from пока не поддерживается.',
            'В текущей сборке адаптера нет.',
            'Формы ошибок этой команды ещё не закреплены.',
            'Предлагаемый ключ --publication пока не поддерживается.',
            'Возможность ещё не реализована.',
            'Текущий CLI сводит такие проверки к invalid_argument.',
            'По умолчанию исходники помещает в src/configuration.',
            'Для внешних наборов не описана.',
            'При ошибке текущий failed-payload содержит applied: false.',
        ]:
            with self.subTest(text=text):
                self.assertTrue(violations(text))

    def test_ready_remote_publication_needs_no_local_web_tools(self):
        subprocess.run(['node', '-e', r"""
const fs = require('fs'), vm = require('vm'), assert = require('assert');
const sandbox = {window: {}};
vm.runInNewContext(fs.readFileSync(process.argv[1], 'utf8'), sandbox);
const model = sandbox.window.RUNNER_DATA;
for (const format of model.AXES.format) for (const type of model.AXES.type)
  for (const target of model.AXES.target) {
    const rows = model.compute({format: format.id, type: type.id, target: target.id, tools: {}});
    const launch = rows.find(row => row.s.id === 'launch-web');
    assert.equal(launch.status, 'ok');
    assert.equal(launch.selected.key, 'browser');
    assert(launch.config.includes('web.url'));
    // Opening an existing URL does not imply permission or ability to publish it.
    if (target.id !== 'standalone')
      assert.equal(rows.find(row => row.s.id === 'publish').status, 'tool');
  }
""", str(ROOT / 'data.js')], check=True)

    def test_preserves_target_constraints_and_runtime_state(self):
        for text in [
            'Если --project-dir не задан, использует текущий каталог.',
            'ibsrv запущен вне раннера и им не управляется.',
            'Перед ibcmd остановите серверные процессы кластера; сеансов недостаточно.',
            'Автономный сервер не поддерживает обычное приложение.',
            'Значение не поддерживается выбранным режимом.',
            'База объявлена, но памяти обмена ещё нет; сравнивать пока не с чем.',
            'План подготовлен. --dry-run не изменяет базу.',
            'Бейдж «План» расположен рядом с названием метода.',
            'v8-runner clone --source-dir src/configuration',
        ]:
            with self.subTest(text=text):
                self.assertFalse(violations(text))


if __name__ == '__main__':
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(RegressionTests)
    if not unittest.TextTestRunner(verbosity=1).run(suite).wasSuccessful():
        raise SystemExit(1)
    check_site()
