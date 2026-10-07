#!/usr/bin/env python3
"""Сверка «сайт = матрица»: цепочки умолчаний сайта против матрицы кода.

Сайт описывает целевое состояние, код — текущее. Пока они расходятся, каждое известное
расхождение записано строкой в `scripts/site_matrix_known.txt` с номером задачи, которая
его снимает. Скрипт падает, если:

- найдено расхождение, которого нет в списке;
- строка списка больше не расходится или расходится иначе, чем записано;
- таблица `architecture.html#d-ops` называет не тех исполнителей, что `target()` в `data.js`;
- сценарий сайта или строка таблицы не сопоставлены, или у операции матрицы нет сценария.

Источники:
- матрица — артефакт `docs/schemas/capability-matrix.json`, порождённый из
  `src/domain/capability.rs` тестом `generated_capability_matrix_is_current`
  (`UPDATE_CAPABILITY_MATRIX=1 cargo test --bin v8-runner generated_capability_matrix_is_current`);
  тест держит артефакт равным коду, скрипт исходник Rust не читает;
- сайт — ветка `target()` из `docs/site/data.js`, вычисляется в `node`.

Что именно сверяется:
- цепочка умолчаний: клетка сайта — порядок, в котором раннер пробует исполнителей,
  поэтому с ней сравниваются только реализованные исполнители строки матрицы в её
  порядке. Экспериментальный (только по ключу `providers.*`) в цепочку не входит; в
  сообщении и в списке известных он показан с пометкой «~»;
- только срез формата DESIGNER и типа CONFIGURATION при всех инструментах в наличии;
- если `applies()` сценария отказывает для цели, цепочка сайта на ней пустая;
- с матрицей сравниваются только её исполнители: `rac`, EDT, клиент и браузер нет. В
  сверке таблицы с `target()` участвуют все исполнители `data.js`, кроме клиента и
  браузера — таблица называет их прозой, а не меткой.

Запуск: `python3 scripts/site_matrix.py` из любого каталога. Нужен `node`.
Контрольные случаи сверки — `tests/site_matrix_cases.py`.
"""

from __future__ import annotations

import html
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MATRIX_JSON = ROOT / "docs" / "schemas" / "capability-matrix.json"
DATA_JS = ROOT / "docs" / "site" / "data.js"
ARCHITECTURE_HTML = ROOT / "docs" / "site" / "architecture.html"
KNOWN = ROOT / "scripts" / "site_matrix_known.txt"
REGENERATE = "UPDATE_CAPABILITY_MATRIX=1 cargo test --bin v8-runner generated_capability_matrix_is_current"

TARGETS = ("file", "cluster", "standalone")

# Сценарий сайта → операции матрицы, которые он исполняет.
SCENARIO_OPERATIONS = {
    "infobase-create": ("infobase.create",),
    "push": ("push",),
    "pull": ("pull",),
    "upload": ("upload",),
    "make": ("make",),
    "convert": ("convert",),
    "download": ("download",),
    "ib-dump": ("infobase.dump", "infobase.restore"),
    "extensions": ("extensions",),
    "check": ("syntax",),
    "publish": ("publish",),
}

# Сценарии, у которых в матрице строки нет: исполнитель у них один или его выбирает не
# матрица. Новый сценарий сайта обязан попасть в один из двух словарей.
SCENARIOS_WITHOUT_ROW = {
    "status",
    "init",
    "clone",
    "apply",
    "reset",
    "diff",
    "sessions",
    "test",
    "launch",
    "launch-web",
}

# Строка таблицы `#d-ops` → сценарий `data.js`. Новая строка таблицы обязана попасть сюда.
TABLE_ROWS = {
    "infobase create": "infobase-create",
    "push": "push",
    "apply": "apply",
    "reset": "reset",
    "upload": "upload",
    "pull": "pull",
    "download": "download",
    "diff": "diff",
    "make": "make",
    "convert": "convert",
    "extensions": "extensions",
    "infobase dump · restore": "ib-dump",
    "check": "check",
    "sessions": "sessions",
    "test": "test",
    "launch": "launch",
    "publish": "publish",
}

# Исполнители, которых таблица называет прозой, а не меткой.
TABLE_PROSE_PROVIDERS = {"client", "browser"}

Row = list[tuple[str, bool]]
Chains = dict[str, dict[str, list[str]]]
Known = dict[tuple[str, str], tuple[str, str, str]]


class Failure(Exception):
    pass


# --- матрица кода -----------------------------------------------------------------


def load_matrix(text: str) -> tuple[dict[tuple[str, str], Row], set[str]]:
    """(операция, цель) → [(исполнитель, реализован)] в порядке строки; все исполнители."""
    try:
        document = json.loads(text)
        providers = set(document["providers"])
        matrix: dict[tuple[str, str], Row] = {}
        for operation, by_target in document["operations"].items():
            for target in TARGETS:
                matrix[(operation, target)] = [
                    (entry["provider"], bool(entry["implemented"])) for entry in by_target[target]
                ]
    except (ValueError, KeyError, TypeError) as error:
        raise Failure(f"{MATRIX_JSON.name} не разобран ({error}); перепородите: {REGENERATE}") from error
    return matrix, providers


# --- сайт -------------------------------------------------------------------------

NODE_PROGRAM = r"""
const fs = require('fs');
const vm = require('vm');
const sandbox = { window: {} };
vm.createContext(sandbox);
vm.runInContext(fs.readFileSync(process.argv[1], 'utf8'), sandbox, { filename: 'data.js' });
const data = sandbox.window.RUNNER_DATA;
const tools = {};
for (const tool of data.AXES.tools) tools[tool.id] = true;
tools.browser = true;
const chains = {};
for (const s of data.SCENARIOS) {
  chains[s.id] = {};
  for (const target of ['file', 'cluster', 'standalone']) {
    const ctx = { format: 'DESIGNER', type: 'CONFIGURATION', target: target, tools: tools, mode: 'target' };
    const blocked = s.applies(ctx);
    let chain = [];
    if (!blocked) {
      const r = s.target.call(s, ctx);
      chain = (r && r.chain ? r.chain : []).map(function (p) { return p.key; });
    }
    chains[s.id][target] = chain;
  }
}
const providers = Object.keys(data.PROVIDERS).map(function (k) { return data.PROVIDERS[k].key; });
process.stdout.write(JSON.stringify({ chains: chains, providers: providers }));
"""


def site_chains() -> tuple[Chains, set[str]]:
    """Цепочки `target()` по сценариям и целям; ключи всех исполнителей `data.js`."""
    node = shutil.which("node")
    if not node:
        raise Failure("нужен node: ветку target() из docs/site/data.js вычисляет он")
    result = subprocess.run(
        [node, "-e", NODE_PROGRAM, str(DATA_JS)],
        capture_output=True,
        text=True,
        encoding="utf-8",
        check=False,
    )
    if result.returncode != 0:
        raise Failure(f"node не вычислил {DATA_JS.name}:\n{result.stderr}")
    out = json.loads(result.stdout)
    return out["chains"], set(out["providers"])


def table_chains(text: str, comparable: set[str]) -> Chains:
    """Метки исполнителей в клетках таблицы `#d-ops`, по порядку, по сценариям."""
    section = re.search(r'<h2 id="d-ops">.*?<tbody>(.*?)</tbody>', text, re.S)
    if not section:
        raise Failure(f"{ARCHITECTURE_HTML.name}: не найдена таблица #d-ops")
    out: Chains = {}
    rows = re.findall(r"<tr\b[^>]*>(.*?)</tr>", section.group(1), re.S)
    for number, row in enumerate(rows, 1):
        cells = re.findall(r"<td\b([^>]*)>(.*?)</td>", row, re.S)
        if not cells:
            raise Failure(f"{ARCHITECTURE_HTML.name}: строка {number} тела таблицы #d-ops без клеток <td>")
        name = html.unescape(re.sub(r"<[^>]+>", "", cells[0][1])).strip()
        if name not in TABLE_ROWS:
            raise Failure(
                f"{ARCHITECTURE_HTML.name}: строка «{name}» таблицы #d-ops не сопоставлена "
                "со сценарием: впишите её в TABLE_ROWS"
            )
        if TABLE_ROWS[name] in out:
            raise Failure(f"{ARCHITECTURE_HTML.name}: строка «{name}» таблицы #d-ops встречается дважды")
        values: list[list[str]] = []
        for attrs, cell in cells[1:]:
            pills = [
                html.unescape(p).strip()
                for p in re.findall(r'<span class="pill [^"]*">(.*?)</span>', cell)
            ]
            chain = [p for p in pills if p in comparable]
            span = re.search(r'colspan="(\d+)"', attrs)
            values.extend([chain] * (int(span.group(1)) if span else 1))
        if len(values) != len(TARGETS):
            raise Failure(f"{ARCHITECTURE_HTML.name}: в строке «{name}» не три цели")
        out[TABLE_ROWS[name]] = dict(zip(TARGETS, values))
    missing = sorted(set(TABLE_ROWS.values()) - set(out))
    if missing:
        raise Failure(f"{ARCHITECTURE_HTML.name}: в #d-ops нет строк {', '.join(missing)}")
    return out


# --- сверка -----------------------------------------------------------------------


def render(chain: list[str]) -> str:
    return " ".join(chain) if chain else "-"


def render_row(row: Row) -> str:
    """Строка матрицы целиком; «~» — только по ключу `providers.*`."""
    return render([provider if implemented else provider + "~" for provider, implemented in row])


def default_chain(row: Row) -> list[str]:
    """Цепочка умолчаний: реализованные исполнители в порядке строки."""
    return [provider for provider, implemented in row if implemented]


def read_known(text: str) -> Known:
    known: Known = {}
    line_pattern = re.compile(
        r"^(?P<op>\S+) (?P<target>\S+) \| сайт: (?P<site>[^|]+?) \| матрица: (?P<matrix>[^|]+?) \| (?P<issue>#\d+)$"
    )
    for number, line in enumerate(text.splitlines(), 1):
        if not line.strip() or line.startswith("#"):
            continue
        found = line_pattern.match(line)
        if not found:
            raise Failure(f"{KNOWN.name}:{number}: строка не по форме: {line}")
        key = (found["op"], found["target"])
        if key in known:
            raise Failure(f"{KNOWN.name}:{number}: {key[0]} {key[1]} записана дважды")
        known[key] = (found["site"], found["matrix"], found["issue"])
    return known


def compare(
    matrix: dict[tuple[str, str], Row],
    providers: set[str],
    site: Chains,
    table: Chains,
    known: Known,
) -> list[str]:
    """Перечень расхождений; пустой — сайт и матрица согласованы."""
    problems: list[str] = []

    for scenario in sorted(set(site) - set(SCENARIO_OPERATIONS) - SCENARIOS_WITHOUT_ROW):
        problems.append(
            f"сценарий `{scenario}` из data.js не сопоставлен с матрицей: "
            "впишите его в SCENARIO_OPERATIONS или SCENARIOS_WITHOUT_ROW"
        )
    for scenario in sorted((set(SCENARIO_OPERATIONS) | SCENARIOS_WITHOUT_ROW) - set(site)):
        problems.append(f"сценария `{scenario}` в data.js нет: уберите его из site_matrix.py")

    mapped_operations = {op for ops in SCENARIO_OPERATIONS.values() for op in ops}
    for operation in sorted({op for op, _ in matrix} - mapped_operations):
        problems.append(f"операция матрицы `{operation}` не сопоставлена со сценарием сайта")

    for scenario, by_target in sorted(table.items()):
        for target in TARGETS:
            expected = [
                p for p in site.get(scenario, {}).get(target, []) if p not in TABLE_PROSE_PROVIDERS
            ]
            if by_target[target] != expected:
                problems.append(
                    f"architecture.html#d-ops «{scenario}» {target}: таблица "
                    f"{render(by_target[target])}, data.js target() {render(expected)}"
                )

    seen: set[tuple[str, str]] = set()
    for scenario, operations in sorted(SCENARIO_OPERATIONS.items()):
        for operation in operations:
            for target in TARGETS:
                key = (operation, target)
                if key not in matrix:
                    problems.append(f"в матрице нет пары {operation} {target}")
                    continue
                chain = [p for p in site.get(scenario, {}).get(target, []) if p in providers]
                site_chain = render(chain)
                row = matrix[key]
                matrix_chain = render_row(row)
                entry = known.get(key)
                if entry:
                    seen.add(key)
                if chain == default_chain(row):
                    if entry:
                        problems.append(
                            f"{operation} {target}: сайт и цепочка умолчаний матрицы совпадают "
                            f"({site_chain}) — уберите строку {entry[2]} из {KNOWN.name}"
                        )
                    continue
                if not entry:
                    problems.append(
                        f"{operation} {target}: сайт {site_chain}, матрица {matrix_chain} — "
                        "новое расхождение; приведите сайт или код к одному, либо внесите строку "
                        f"с номером задачи в {KNOWN.name}"
                    )
                elif (entry[0], entry[1]) != (site_chain, matrix_chain):
                    problems.append(
                        f"{operation} {target}: записано «сайт: {entry[0]} | матрица: {entry[1]}» "
                        f"({entry[2]}), сейчас «сайт: {site_chain} | матрица: {matrix_chain}» — "
                        "обновите строку"
                    )
    for key in sorted(set(known) - seen):
        problems.append(f"{KNOWN.name}: строка {key[0]} {key[1]} не называет пару операции и цели из сверки")
    return problems


def main() -> int:
    try:
        matrix, providers = load_matrix(MATRIX_JSON.read_text(encoding="utf-8"))
        site, site_providers = site_chains()
        table = table_chains(
            ARCHITECTURE_HTML.read_text(encoding="utf-8"), site_providers - TABLE_PROSE_PROVIDERS
        )
        known = read_known(KNOWN.read_text(encoding="utf-8"))
    except (Failure, OSError) as error:
        print(f"site_matrix: {error}", file=sys.stderr)
        return 2

    problems = compare(matrix, providers, site, table, known)
    if problems:
        print("Сайт и матрица расходятся:", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        print(
            "Матрица — docs/schemas/capability-matrix.json из src/domain/capability.rs, сайт — "
            "target() в docs/site/data.js и таблица docs/site/architecture.html#d-ops; "
            "сверяется цепочка умолчаний, «~» — исполнитель только по ключу providers.*.",
            file=sys.stderr,
        )
        return 1
    print(f"site_matrix: сайт = матрица с учётом {len(known)} известных расхождений из {KNOWN.name}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
