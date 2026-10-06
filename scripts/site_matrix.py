#!/usr/bin/env python3
"""Сверка «сайт = матрица»: цепочки исполнителей сайта против матрицы кода.

Сайт описывает целевое состояние, код — текущее. Пока они расходятся, каждое известное
расхождение записано строкой в `scripts/site_matrix_known.txt` с номером задачи, которая
его снимает. Скрипт падает, если:

- найдено расхождение, которого нет в списке;
- строка списка больше не расходится или расходится иначе, чем записано;
- таблица `architecture.html#d-ops` называет не тех исполнителей, что `target()` в `data.js`;
- сценарий сайта не сопоставлен с операцией матрицы и не объявлен операцией без строки.

Источники:
- матрица — `capabilities()` в `src/domain/capability.rs`, разбирается из исходника;
  незнакомая форма записи — отказ разбора, а не молчаливый пропуск;
- сайт — ветка `target()` из `docs/site/data.js`, вычисляется в `node` при всех
  инструментах в наличии; исполнители вне матрицы (`rac`, клиент, EDT, браузер) не
  сравниваются.

Запуск: `python3 scripts/site_matrix.py` из любого каталога. Нужен `node`.
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
CAPABILITY_RS = ROOT / "src" / "domain" / "capability.rs"
DATA_JS = ROOT / "docs" / "site" / "data.js"
ARCHITECTURE_HTML = ROOT / "docs" / "site" / "architecture.html"
KNOWN = ROOT / "scripts" / "site_matrix_known.txt"

TARGETS = ("file", "cluster", "standalone")

# Сценарий сайта → операции матрицы, которые он исполняет.
SCENARIO_OPERATIONS = {
    "infobase-create": ("infobase.create",),
    "push": ("push",),
    "pull": ("pull",),
    "upload": ("upload",),
    "make": ("make",),
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
    "convert",
}

# Строка таблицы `#d-ops` → сценарий `data.js`.
TABLE_ROWS = {
    "infobase create": "infobase-create",
    "push": "push",
    "pull": "pull",
    "upload": "upload",
    "make": "make",
    "download": "download",
    "infobase dump · restore": "ib-dump",
    "extensions": "extensions",
    "check": "check",
    "publish": "publish",
}


class Failure(Exception):
    pass


# --- матрица кода -----------------------------------------------------------------


def strip_comments(source: str) -> str:
    return re.sub(r"//[^\n]*", "", source)


def enum_names(source: str, enum: str) -> dict[str, str]:
    """`Self::Variant => "name"` из `as_str` у `impl <enum>`."""
    match = re.search(
        r"impl " + enum + r" \{.*?pub const fn as_str\(self\) -> &'static str \{\s*match self \{(.*?)\}",
        source,
        re.S,
    )
    if not match:
        raise Failure(f"{CAPABILITY_RS.name}: не найден `{enum}::as_str`")
    names = dict(re.findall(r'Self::(\w+)\s*=>\s*"([^"]+)"', match.group(1)))
    if not names:
        raise Failure(f"{CAPABILITY_RS.name}: `{enum}::as_str` пуст")
    return names


def leftover(text: str, pattern: str) -> str:
    return re.sub(pattern, "", text, flags=re.S).strip()


def parse_matrix() -> dict[tuple[str, str], list[tuple[str, bool]]]:
    """(операция, цель) → [(исполнитель, реализован)] в порядке строки."""
    source = strip_comments(CAPABILITY_RS.read_text(encoding="utf-8"))
    providers = enum_names(source, "Provider")
    operations = enum_names(source, "Operation")
    targets = enum_names(source, "TargetKind")

    body = re.search(
        r"pub fn capabilities\(operation: Operation, target: TargetKind\) -> &'static \[Capability\] \{(.*?)\n\}",
        source,
        re.S,
    )
    if not body:
        raise Failure(f"{CAPABILITY_RS.name}: не найдена `capabilities()`")
    body = body.group(1)

    rows: dict[str, list[tuple[str, bool]]] = {"&[]": []}
    const_pattern = r"const (\w+): &\[Capability\] = &\[(.*?)\];"
    for name, items in re.findall(const_pattern, body, re.S):
        entry = r"(implemented|experimental)\((\w+),\s*\w+\)"
        if leftover(items, entry + r"\s*,?"):
            raise Failure(f"{CAPABILITY_RS.name}: незнакомая запись в `{name}`: {items.strip()}")
        row = []
        for kind, provider in re.findall(entry, items):
            if provider not in providers:
                raise Failure(f"{CAPABILITY_RS.name}: `{name}` называет неизвестного `{provider}`")
            row.append((providers[provider], kind == "implemented"))
        rows[name] = row

    match_body = re.search(r"match \(operation, target\) \{(.*)\}\s*$", body, re.S)
    if not match_body:
        raise Failure(f"{CAPABILITY_RS.name}: не найден `match (operation, target)`")
    arm = re.compile(
        r"\(\s*(?P<ops>[\w:|\s]+?)\s*,\s*(?P<targets>[\w:|\s]+?)\s*,?\s*\)\s*=>\s*"
        r"(?:\{\s*(?P<block>\w+)\s*\}|(?P<value>\w+|&\[\]))\s*,?",
        re.S,
    )
    arms_text = match_body.group(1)
    if leftover(arms_text, arm.pattern):
        raise Failure(
            f"{CAPABILITY_RS.name}: незнакомая ветка в `match (operation, target)`: "
            + leftover(arms_text, arm.pattern)[:200]
        )

    def alternatives(text: str, prefix: str, names: dict[str, str]) -> list[str]:
        if text.strip() == "_":
            return list(names.values())
        out = []
        for part in text.split("|"):
            part = part.strip()
            if not part.startswith(prefix + "::") or part[len(prefix) + 2 :] not in names:
                raise Failure(f"{CAPABILITY_RS.name}: незнакомый образец `{part}`")
            out.append(names[part[len(prefix) + 2 :]])
        return out

    matrix: dict[tuple[str, str], list[tuple[str, bool]]] = {}
    for found in arm.finditer(arms_text):
        value = found.group("block") or found.group("value")
        if value not in rows:
            raise Failure(f"{CAPABILITY_RS.name}: ветка ссылается на неизвестную строку `{value}`")
        for operation in alternatives(found.group("ops"), "Operation", operations):
            for target in alternatives(found.group("targets"), "TargetKind", targets):
                matrix.setdefault((operation, target), rows[value])
    missing = [
        f"{operation} {target}"
        for operation in operations.values()
        for target in targets.values()
        if (operation, target) not in matrix
    ]
    if missing:
        raise Failure(f"{CAPABILITY_RS.name}: матрица не покрывает {', '.join(missing)}")
    return matrix


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
const out = {};
for (const s of data.SCENARIOS) {
  out[s.id] = {};
  for (const target of ['file', 'cluster', 'standalone']) {
    const ctx = { format: 'DESIGNER', type: 'CONFIGURATION', target: target, tools: tools, mode: 'target' };
    const blocked = s.applies(ctx);
    let chain = [];
    if (!blocked) {
      const r = s.target.call(s, ctx);
      chain = (r && r.chain ? r.chain : []).map(function (p) { return p.key; });
    }
    out[s.id][target] = chain;
  }
}
process.stdout.write(JSON.stringify(out));
"""


def site_chains() -> dict[str, dict[str, list[str]]]:
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
    return json.loads(result.stdout)


def table_chains(known_providers: set[str]) -> dict[str, dict[str, list[str]]]:
    """Исполнители матрицы в клетках таблицы `#d-ops`, по порядку."""
    text = ARCHITECTURE_HTML.read_text(encoding="utf-8")
    section = re.search(r'<h2 id="d-ops">.*?<tbody>(.*?)</tbody>', text, re.S)
    if not section:
        raise Failure(f"{ARCHITECTURE_HTML.name}: не найдена таблица #d-ops")
    out: dict[str, dict[str, list[str]]] = {}
    for row in re.findall(r"<tr>(.*?)</tr>", section.group(1), re.S):
        cells = re.findall(r"<td([^>]*)>(.*?)</td>", row, re.S)
        name = html.unescape(re.sub(r"<[^>]+>", "", cells[0][1])).strip()
        if name not in TABLE_ROWS:
            continue
        values: list[list[str]] = []
        for attrs, cell in cells[1:]:
            pills = [
                html.unescape(p).strip()
                for p in re.findall(r'<span class="pill [^"]*">(.*?)</span>', cell)
            ]
            chain = [p for p in pills if p in known_providers]
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


def render_row(row: list[tuple[str, bool]]) -> str:
    return render([provider if implemented else provider + "~" for provider, implemented in row])


def read_known() -> dict[tuple[str, str], tuple[str, str, str]]:
    known: dict[tuple[str, str], tuple[str, str, str]] = {}
    line_pattern = re.compile(
        r"^(?P<op>\S+) (?P<target>\S+) \| сайт: (?P<site>[^|]+?) \| матрица: (?P<matrix>[^|]+?) \| (?P<issue>#\d+)$"
    )
    for number, line in enumerate(KNOWN.read_text(encoding="utf-8").splitlines(), 1):
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


def main() -> int:
    try:
        matrix = parse_matrix()
        source = strip_comments(CAPABILITY_RS.read_text(encoding="utf-8"))
        providers = set(enum_names(source, "Provider").values())
        site = site_chains()
        table = table_chains(providers)
        known = read_known()
    except Failure as error:
        print(f"site_matrix: {error}", file=sys.stderr)
        return 2

    problems: list[str] = []

    unmapped = sorted(set(site) - set(SCENARIO_OPERATIONS) - SCENARIOS_WITHOUT_ROW)
    for scenario in unmapped:
        problems.append(
            f"сценарий `{scenario}` из data.js не сопоставлен с матрицей: "
            "впишите его в SCENARIO_OPERATIONS или SCENARIOS_WITHOUT_ROW"
        )
    for scenario in sorted((set(SCENARIO_OPERATIONS) | SCENARIOS_WITHOUT_ROW) - set(site)):
        problems.append(f"сценария `{scenario}` в data.js нет: уберите его из {Path(__file__).name}")

    mapped_operations = {op for ops in SCENARIO_OPERATIONS.values() for op in ops}
    for operation in sorted({op for op, _ in matrix} - mapped_operations):
        problems.append(f"операция матрицы `{operation}` не сопоставлена со сценарием сайта")

    for scenario, by_target in sorted(table.items()):
        for target in TARGETS:
            expected = [p for p in site.get(scenario, {}).get(target, []) if p in providers]
            if by_target[target] != expected:
                problems.append(
                    f"architecture.html#d-ops «{scenario}» {target}: таблица {render(by_target[target])}, "
                    f"data.js target() {render(expected)}"
                )

    seen: set[tuple[str, str]] = set()
    for scenario, operations in sorted(SCENARIO_OPERATIONS.items()):
        for operation in operations:
            for target in TARGETS:
                key = (operation, target)
                site_chain = render([p for p in site.get(scenario, {}).get(target, []) if p in providers])
                row = matrix[key]
                matrix_chain = render_row(row)
                diverges = site_chain != render([provider for provider, _ in row])
                entry = known.get(key)
                if entry:
                    seen.add(key)
                if not diverges:
                    if entry:
                        problems.append(
                            f"{operation} {target}: сайт и матрица совпадают ({site_chain}) — "
                            f"уберите строку {entry[2]} из {KNOWN.name}"
                        )
                    continue
                if not entry:
                    problems.append(
                        f"{operation} {target}: сайт {site_chain}, матрица {matrix_chain} — "
                        f"новое расхождение; приведите сайт или код к одному, либо внесите строку "
                        f"с номером задачи в {KNOWN.name}"
                    )
                elif (entry[0], entry[1]) != (site_chain, matrix_chain):
                    problems.append(
                        f"{operation} {target}: записано «сайт: {entry[0]} | матрица: {entry[1]}» ({entry[2]}), "
                        f"сейчас «сайт: {site_chain} | матрица: {matrix_chain}» — обновите строку"
                    )
    for key in sorted(set(known) - seen):
        problems.append(f"{KNOWN.name}: строка {key[0]} {key[1]} не называет пару операции и цели из сверки")

    if problems:
        print("Сайт и матрица расходятся:", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        print(
            "Матрица — src/domain/capability.rs, сайт — target() в docs/site/data.js и "
            "таблица docs/site/architecture.html#d-ops; «~» — исполнитель только по ключу providers.*.",
            file=sys.stderr,
        )
        return 1
    print(f"site_matrix: сайт = матрица с учётом {len(known)} известных расхождений из {KNOWN.name}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
