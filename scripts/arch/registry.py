#!/usr/bin/env python3
"""Read the architecture registry and render its index.

Модель реестра заимствована из проекта Unica (`arch/`): три реестра, одна запись —
один файл, символ и путь выводятся друг из друга.

Three registries share one record shape: a markdown file that opens with a
front-matter block of props and continues with prose. The symbol and the path
derive from each other, so navigation never needs the index — the index exists
so a reader can hold the whole registry in one screen and grep it in one pass.

The front-matter parser is a deliberate subset of YAML: scalars, flat lists and
`null`. A registry record that needs more structure than that is a record that
outgrew its purpose.

Usage:
    registry.py                 # печатает индекс в stdout
    registry.py --write-index   # записывает spec/arch/index.md
    registry.py --check         # молча выходит с 1, если индекс устарел
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
ARCH_ROOT = REPO_ROOT / "spec" / "arch"
INDEX_PATH = ARCH_ROOT / "index.md"

KIND_BY_DIR = {"decisions": "decision", "invariants": "invariant", "contracts": "contract"}
SYMBOL_PREFIX = {"decision": "DEC", "invariant": "INV", "contract": "CTR"}

REQUIRED_PROPS = {
    "decision": ("id", "status", "governs", "realized"),
    "invariant": ("id", "status", "governs", "decision", "check", "scope"),
    "contract": (
        "id",
        "status",
        "governs",
        "version",
        "decision",
        "producer",
        # Форма контракта закреплена файлом: схемой, снимком или фикстурой. Без него
        # запись описывает форму словами, и сломать её можно, не задев ни строчки
        # документации.
        "artifact",
        "consumers",
        "check",
        "scope",
    ),
}

# Кто заметит нарушение: потребитель или только мы. Ось решает не предмет
# записи, а адресат обещания, и от неё зависит, чем правка оплачивается.
GOVERNS = ("product", "process")

FRONT_MATTER = re.compile(r"\A---\n(.*?)\n---\n(.*)\Z", re.S)
DECISION_FILENAME = re.compile(r"\A(\d{4}-\d{2}-\d{2})-([a-z0-9-]+)\.md\Z")

EXAMPLE_HEADING = re.compile(r"^## Пример\s*$", re.M)
FENCED_BLOCK = re.compile(r"^```[a-z]*\n(.*?)^```\s*$", re.M | re.S)

# A symbol becomes a filename, and Windows still refuses these as base names
# whatever the extension follows. `CON` was the first contract prefix and made
# the whole tree impossible to check out on Windows.
DOS_DEVICE_NAMES = frozenset(
    ["CON", "PRN", "AUX", "NUL"]
    + [f"COM{digit}" for digit in range(10)]
    + [f"LPT{digit}" for digit in range(10)]
)


@dataclass
class Record:
    id: str
    kind: str
    path: Path
    props: dict = field(default_factory=dict)
    body: str = ""

    @property
    def summary(self) -> str:
        """The first heading, or the first non-empty line without markup."""
        for line in self.body.splitlines():
            line = line.strip()
            if not line:
                continue
            return line.lstrip("# ").strip()
        return ""

    @property
    def relative(self) -> str:
        return self.path.relative_to(ARCH_ROOT).as_posix()


def parse_front_matter(text: str) -> tuple[dict, str]:
    """Split a record into its props and its body.

    Supports `key: scalar`, `key: [a, b]` and `key: null`. Anything else is a
    parse error rather than a silent partial read.
    """
    match = FRONT_MATTER.match(text)
    if not match:
        raise ValueError("record does not open with a front-matter block")
    props: dict = {}
    block_key: str | None = None
    for number, line in enumerate(match.group(1).splitlines(), start=1):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        # Блочный список продолжает ключ над собой. Продолжить можно только
        # список, открытый пустым значением, поэтому одиночный `- item` — это
        # испорченная запись, а не молча усыновлённый сирота.
        if line.lstrip().startswith("- "):
            if block_key is None:
                raise ValueError(f"front matter line {number} starts a list with no key")
            props[block_key].append(line.lstrip()[2:].strip())
            continue
        block_key = None
        if ":" not in line:
            raise ValueError(f"front matter line {number} is not `key: value`: {line!r}")
        key, _, raw = line.partition(":")
        key, raw = key.strip(), raw.strip()
        if raw.startswith("[") and raw.endswith("]"):
            inner = raw[1:-1].strip()
            props[key] = [item.strip() for item in inner.split(",") if item.strip()]
        elif raw == "":
            props[key] = []
            block_key = key
        elif raw in ("null", "~"):
            props[key] = None
        else:
            props[key] = raw
    return props, match.group(2)


def example_block(body: str) -> str:
    """Тело первого блока кода в разделе «Пример», или пустая строка.

    Раздел обязателен у контракта: схема говорит, что допустимо, а пример —
    что потребитель увидит на самом деле. Проверка `tests/arch_registry.rs`
    прогоняет его через закреплённую форму, поэтому устареть молча он не может.
    """
    heading = EXAMPLE_HEADING.search(body)
    if heading is None:
        return ""
    block = FENCED_BLOCK.search(body, heading.end())
    return block.group(1) if block else ""


def evidence_names(value: object) -> list[str]:
    """Каждый адрес `path::declaration`, названный пропом `check` или `realized`.

    Одно правило часто держат несколько проверок. Принуждение к единственному
    имени не делало правило проще: оно рождало обёртку, которая звала настоящие
    проверки и повторяла работу, уже сделанную харнессом. Проп принимает список,
    и запись называет ровно тот набор, который её держит.
    """
    if value is None or value == "":
        return []
    if isinstance(value, list):
        return [str(item) for item in value]
    return [str(value)]


def records(root: Path = ARCH_ROOT) -> list[Record]:
    """Every record of every registry, ordered by symbol."""
    found: list[Record] = []
    for directory, kind in KIND_BY_DIR.items():
        base = root / directory
        if not base.is_dir():
            continue
        for path in sorted(base.glob("*.md")):
            props, body = parse_front_matter(path.read_text(encoding="utf-8"))
            identifier = props.get("id") or ""
            found.append(Record(id=identifier, kind=kind, path=path, props=props, body=body))
    return sorted(found, key=lambda record: record.id)


def expected_symbol(record: Record) -> str:
    """The symbol a record's own path demands."""
    if record.kind == "decision":
        match = DECISION_FILENAME.match(record.path.name)
        if not match:
            return ""
        date, slug = match.groups()
        return f"DEC.{date}.{slug.upper()}"
    return record.path.stem


def validation_errors(found: list[Record]) -> list[str]:
    """Return violations of the published record schema."""
    by_id = {record.id: record for record in found}
    errors: list[str] = []
    for record in found:
        for key in REQUIRED_PROPS[record.kind]:
            # Правило, у которого фальсификатор ещё не написан, заводится со
            # `status: planned` и `check: null`. Так долг виден в индексе, а не
            # прячется в ненаписанной записи.
            realized_may_be_absent = (
                record.kind == "decision"
                and key == "realized"
                and record.props.get("status") in {"planned", "superseded"}
            ) or (
                record.kind in {"invariant", "contract"}
                and key == "check"
                and record.props.get("status") == "planned"
            )
            if (
                key not in record.props
                or record.props[key] == ""
                or record.props[key] == []
                or (record.props[key] is None and not realized_may_be_absent)
            ):
                errors.append(f"{record.relative}: missing prop `{key}`")
            elif key in ("check", "realized"):
                if any(not name.strip() for name in evidence_names(record.props[key])):
                    errors.append(f"{record.relative}: `{key}` has a blank entry")

        # Символ и путь восстанавливают друг друга. Имя файла даёт символ, префикс
        # вида — каталог; без второй половины обратный ход не собирается, и
        # `CTR.WIRE.FOO`, лежащий в `invariants/`, не находится ни по символу, ни
        # дважды в индексе как один символ. Разойдясь, путь и символ не подают
        # знака: индекс порождается из тех же записей и повторяет подмену.
        expected = expected_symbol(record)
        if record.kind == "decision" and not expected:
            errors.append(f"{record.relative}: filename must read `<ГГГГ-ММ-ДД>-<slug>.md`")
        elif record.id and record.id != expected:
            errors.append(f"{record.relative}: `id` must read `{expected}`")
        prefix = SYMBOL_PREFIX[record.kind]
        if record.id and not record.id.startswith(f"{prefix}."):
            errors.append(f"{record.relative}: `id` must open with `{prefix}.`")

        # Символ становится именем файла, и базовое имя из DOS_DEVICE_NAMES
        # Windows отказывается создавать с любым расширением: не выкладывается
        # уже всё дерево, а не одна запись.
        base = record.path.name.split(".", 1)[0]
        if base.upper() in DOS_DEVICE_NAMES:
            errors.append(
                f"{record.relative}: `{base}` is a Windows device name; choose another symbol"
            )

        if record.kind in {"invariant", "contract"}:
            list_keys = ("scope",) + (("consumers",) if record.kind == "contract" else ())
            for key in list_keys:
                if not isinstance(record.props.get(key), list) or not record.props[key]:
                    errors.append(f"{record.relative}: `{key}` must be a non-empty list")
            owner = by_id.get(record.props.get("decision"))
            if owner is None or owner.kind != "decision":
                errors.append(f"{record.relative}: decision does not resolve to a decision")
            elif record.props.get("status") == "active" and owner.props.get("status") != "active":
                errors.append(f"{record.relative}: active rule cites a non-active decision")
            elif record.id not in (owner.props.get("establishes") or []):
                errors.append(
                    f"{owner.relative}: does not establish its rule {record.id}"
                )
        if record.kind == "contract":
            artifact = str(record.props.get("artifact", ""))
            if artifact and not (REPO_ROOT / artifact).is_file():
                errors.append(f"{record.relative}: artifact {artifact} does not exist")
            version = str(record.props.get("version", ""))
            if not version.isdecimal() or int(version) < 1:
                errors.append(f"{record.relative}: version must be a positive integer")
            if not example_block(record.body).strip():
                errors.append(
                    f"{record.relative}: no `## Пример` section with a fenced block"
                )
        if record.kind == "decision":
            if "changes" in record.props:
                changed_contracts = record.props.get("changes")
                if not isinstance(changed_contracts, list) or not changed_contracts:
                    errors.append(
                        f"{record.relative}: `changes` must be a list and not empty"
                    )
                else:
                    # `changes` называет опубликованное правило, которое это
                    # решение меняет. Правилом бывает и контракт, и инвариант:
                    # инвариант перечисляет имена поверхности не реже, чем
                    # контракт, и запрет ссылаться на него оставлял такую
                    # правку без объявленной причины.
                    for rule_id in changed_contracts:
                        rule = by_id.get(rule_id)
                        if rule is None:
                            errors.append(
                                f"{record.relative}: changes cites missing rule "
                                f"{rule_id}"
                            )
                        elif rule.kind not in ("contract", "invariant"):
                            errors.append(
                                f"{record.relative}: changes cites a non-rule "
                                f"{rule_id}"
                            )
            # `establishes` is historical on an immutable decision. A later
            # decision may become the current owner of the same mutable rule;
            # the rule -> current owner direction above remains mandatory.
    return errors


def render_index(found: list[Record]) -> str:
    """One line per symbol, sorted, with no fact that props do not carry."""
    kind_ru = {"decision": "решение", "invariant": "инвариант", "contract": "контракт"}
    lines = [
        "<!-- ПОРОЖДАЕТСЯ scripts/arch/registry.py --write-index; руками не правится -->",
        "",
        "# Индекс реестра",
        "",
        "| Символ | Вид | Статус | Проверяется | Суть | Файл |",
        "| --- | --- | --- | --- | --- | --- |",
    ]
    for record in found:
        # Колонка отвечает за оба вида: у решения — есть ли свидетельство
        # реализации, у правила — написан ли фальсификатор. И там и там читатель
        # иначе не отличает принятое от действующего.
        built = ""
        if record.kind == "decision":
            built = "да" if evidence_names(record.props.get("realized")) else "нет"
        else:
            built = "да" if evidence_names(record.props.get("check")) else "нет"
        lines.append(
            f"| `{record.id}` | {kind_ru[record.kind]} · {record.props.get('governs', '')} "
            f"| {record.props.get('status', '')} "
            f"| {built} | {record.summary} | [{record.relative}]({record.relative}) |"
        )
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--write-index", action="store_true")
    parser.add_argument("--check", action="store_true")
    arguments = parser.parse_args(argv)

    found = records()
    errors = validation_errors(found)
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1

    rendered = render_index(found)
    if arguments.write_index:
        INDEX_PATH.write_text(rendered, encoding="utf-8")
        print(f"написано: {INDEX_PATH.relative_to(REPO_ROOT)}")
        return 0
    if arguments.check:
        current = INDEX_PATH.read_text(encoding="utf-8") if INDEX_PATH.is_file() else ""
        if current != rendered:
            print("индекс устарел: перегенерируйте --write-index", file=sys.stderr)
            return 1
        return 0
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
