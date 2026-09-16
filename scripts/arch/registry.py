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

# Перечни, которые README объявляет закрытыми. Гейт читает оба поля точным
# равенством, поэтому значение вне перечня не отвергается само собой: оно
# оказывается «ни тем ни другим», и каждая проверка, что на него ветвится, молча
# перестаёт применяться. Перечень `scope` README называет открытым — его здесь нет.
STATUSES = ("active", "planned", "superseded")
# Кто заметит нарушение: потребитель или только мы. Ось решает не предмет
# записи, а адресат обещания, и от неё зависит, чем правка оплачивается.
GOVERNS = ("product", "process")

# Форма значения, обещанная README наравне со смыслом поля. Проверки ниже читают её
# как известную: `scope` обходят циклом, `superseded-by` сравнивают с символом. Поле
# не той формы не роняет проверку — оно заставляет её отвечать про другое.
# `check` и `realized` не здесь: одно правило держат и один адрес, и список.
LIST_PROPS = ("supersedes", "establishes", "changes", "scope", "consumers")
SINGLE_PROPS = (
    "id",
    "status",
    "governs",
    "version",
    "artifact",
    "producer",
    "decision",
    "superseded-by",
)

FRONT_MATTER = re.compile(r"\A---\n(.*?)\n---\n(.*)\Z", re.S)
# Дата пишется теми же ASCII-цифрами, что и остальное имя: `\d` принимает и
# арабо-индийские, и тогда символ решения собирается из знаков, которых нет ни в
# одной ссылке на него.
DECISION_FILENAME = re.compile(r"\A([0-9]{4}-[0-9]{2}-[0-9]{2})-([a-z0-9-]+)\.md\Z")

EXAMPLE_HEADING = re.compile(r"^## Пример\s*$", re.M)
FENCED_BLOCK = re.compile(r"^```[a-z]*\n(.*?)^```\s*$", re.M | re.S)

# A symbol becomes a filename, and Windows still refuses these as base names
# whatever the extension follows. `CON` was the first contract prefix and made
# the whole tree impossible to check out on Windows.
#
# Digits start at one: the system reserves COM1..COM9 and LPT1..LPT9, and `COM0`
# is an ordinary name. Widening the list back would ban a name nothing refuses.
DOS_DEVICE_NAMES = frozenset(
    ["CON", "PRN", "AUX", "NUL"]
    + [f"COM{digit}" for digit in range(1, 10)]
    + [f"LPT{digit}" for digit in range(1, 10)]
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


def record_from(path: Path, text: str, kind: str) -> Record:
    """One record, assembled from one file's name and text.

    Собирается запись здесь, а не в `records()`, потому что читателей у неё двое:
    обход каталога и фальсификатор, который подсовывает выдуманные записи, не
    трогая диск. Две сборки разошлись бы молча — гейт судил бы одну форму, а
    проверка гейта другую.
    """
    props, body = parse_front_matter(text)
    # Символом `id` бывает только скаляр. Списком он молча ронял разбор всего реестра
    # — `by_id` не берёт в ключи список, — поэтому запись без читаемого символа несёт
    # пустой: о самом поле говорит `shape_errors`, а остальные проверки его не ищут.
    symbol = props.get("id")
    return Record(
        id=symbol if isinstance(symbol, str) else "",
        kind=kind,
        path=path,
        props=props,
        body=body,
    )


def records(root: Path = ARCH_ROOT) -> list[Record]:
    """Every record of every registry, ordered by symbol."""
    found: list[Record] = []
    for directory, kind in KIND_BY_DIR.items():
        base = root / directory
        if not base.is_dir():
            continue
        for path in sorted(base.glob("*.md")):
            found.append(record_from(path, path.read_text(encoding="utf-8"), kind))
    return sorted(found, key=lambda record: record.id)


def expected_symbol(record: Record) -> str:
    """The symbol a record's own filename spells, prefix aside.

    Вторую половину — префикс вида, который называет каталог, — держит отдельная
    проверка в `validation_errors`: путь, у которого половины спорят, одного
    символа не диктует.
    """
    if record.kind == "decision":
        match = DECISION_FILENAME.match(record.path.name)
        if not match:
            return ""
        date, slug = match.groups()
        return f"DEC.{date}.{slug.upper()}"
    return record.path.stem


def shape_errors(record: Record) -> list[str]:
    """Поля, чья форма разошлась с опубликованной.

    README называет форму каждого поля рядом с его смыслом, и проверки ниже читают её
    как известную. Форма не та — и проверка не падает, а отвечает про другое: `in` по
    строке ищет подстроку, `by_id.get` по списку роняет весь разбор. Поэтому форма
    сверяется первой, а всё, что от неё зависит, ниже за неё и прячется.
    """
    wrong: list[str] = []
    for key in LIST_PROPS:
        if key in record.props and not isinstance(record.props[key], list):
            wrong.append(f"{record.relative}: `{key}` must be a list")
    for key in SINGLE_PROPS:
        value = record.props.get(key)
        if not isinstance(value, list):
            continue
        # `key:` без значения разбирается в пустой список. У обязательного поля об
        # этом уже сказано словом «missing», и второе сообщение сказало бы то же.
        if not value and key in REQUIRED_PROPS[record.kind]:
            continue
        wrong.append(f"{record.relative}: `{key}` takes a single value, not a list")
    return wrong


def cited_rule_errors(record: Record, key: str, by_id: dict[str, Record]) -> list[str]:
    """Символы, названные пропом решения, — и то, чем они оказались.

    `establishes` называет правило, выведенное из решения, `changes` — правило, чью
    наблюдаемую форму решение меняет. Оба поля ведут от решения к правилу, и символ,
    за которым записи нет, — обещание, по которому не прийти.
    """
    cited = record.props.get(key)
    if not isinstance(cited, list):
        return []
    wrong: list[str] = []
    for rule_id in cited:
        rule = by_id.get(rule_id) if isinstance(rule_id, str) else None
        if rule is None:
            wrong.append(f"{record.relative}: {key} cites missing rule {rule_id}")
        elif rule.kind not in ("contract", "invariant"):
            wrong.append(f"{record.relative}: {key} cites a non-rule {rule_id}")
    return wrong


def supersession_claims(found: list[Record]) -> dict[tuple[str, str], Record]:
    """Каждая заявленная пара «заменённое решение → преемник» и тот, кто её заявил.

    Заявить замену может любая из двух записей: старая полем `superseded-by`, новая
    перечнем `supersedes`. Пара здесь и есть предмет проверки — иначе одна
    ненаписанная половина стоит двух сообщений, и автор ищет вторую правку там, где
    её нет.
    """
    claims: dict[tuple[str, str], Record] = {}
    for record in found:
        if record.kind != "decision" or not record.id:
            continue
        successor = record.props.get("superseded-by")
        if isinstance(successor, str) and successor:
            claims.setdefault((record.id, successor), record)
        superseded = record.props.get("supersedes")
        if isinstance(superseded, list):
            for old_id in superseded:
                if isinstance(old_id, str) and old_id:
                    claims.setdefault((old_id, record.id), record)
    return claims


def validation_errors(found: list[Record]) -> list[str]:
    """Return violations of the published record schema."""
    by_id = {record.id: record for record in found if record.id}
    errors: list[str] = []

    # Символ называет ровно одну запись. Имя файла и префикс вида его уже диктуют, но
    # порознь: стоит двум видам назваться одним префиксом, и один символ лежит в двух
    # каталогах сразу. Тогда ссылка по нему приводит к той записи, что победила в
    # `by_id`, а индекс печатает на него две строки — обе выглядят правдой.
    first_named_by: dict[str, Record] = {}
    for record in found:
        if not record.id:
            continue
        first = first_named_by.setdefault(record.id, record)
        if first is not record:
            errors.append(
                f"{record.relative}: symbol {record.id} already names {first.relative}"
            )

    for record in found:
        errors.extend(shape_errors(record))
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
        # вида — каталог; без второй половины обратный ход не собирается:
        # `CTR.WIRE.FOO`, лежащий в `invariants/`, не находится по символу, а два
        # таких файла дали бы в индексе две строки на один символ. Разойдясь, путь
        # и символ не подают знака: индекс порождается из тех же записей.
        expected = expected_symbol(record)
        if record.kind == "decision" and not expected:
            errors.append(f"{record.relative}: filename must read `<YYYY-MM-DD>-<slug>.md`")
        elif record.id and record.id != expected:
            errors.append(f"{record.relative}: `id` must read `{expected}`")
        prefix = SYMBOL_PREFIX[record.kind]
        if record.id and not record.id.startswith(f"{prefix}."):
            errors.append(f"{record.relative}: `id` must open with `{prefix}.`")

        # Символ становится именем файла, и базовое имя устройства Windows
        # отказывается создавать с любым расширением: перестаёт выкладываться всё
        # дерево, а не одна запись.
        base = record.path.name.split(".", 1)[0]
        if base.upper() in DOS_DEVICE_NAMES:
            errors.append(
                f"{record.relative}: `{base}` is a Windows device name; choose another symbol"
            )

        # Оба перечня закрыты, и обе оси решают, какие проверки к записи применяются,
        # а не описывают её. Опечатка в них не отвергалась и не бросалась в глаза:
        # индекс печатает значение как есть, и колонка выглядит заполненной.
        for key, published in (("status", STATUSES), ("governs", GOVERNS)):
            value = record.props.get(key)
            if isinstance(value, str) and value and value not in published:
                errors.append(
                    f"{record.relative}: `{key}` must be one of {', '.join(published)}"
                )

        if record.kind in {"invariant", "contract"}:
            owner_id = record.props.get("decision")
            if isinstance(owner_id, str):
                owner = by_id.get(owner_id)
                established = owner.props.get("establishes") if owner else None
                if owner is None or owner.kind != "decision":
                    errors.append(f"{record.relative}: decision does not resolve to a decision")
                elif (
                    record.props.get("status") == "active"
                    and owner.props.get("status") != "active"
                ):
                    errors.append(f"{record.relative}: active rule cites a non-active decision")
                # Перечень решения обходится членством, а не подстрокой, и только если
                # он перечень: у правила без читаемого символа искать нечего.
                elif record.id and isinstance(established, list) and record.id not in established:
                    errors.append(
                        f"{owner.relative}: does not establish its rule {record.id}"
                    )
        if record.kind == "contract":
            artifact = record.props.get("artifact")
            if isinstance(artifact, str) and artifact and not (REPO_ROOT / artifact).is_file():
                errors.append(f"{record.relative}: artifact {artifact} does not exist")
            version = record.props.get("version")
            if isinstance(version, str) and (not version.isdecimal() or int(version) < 1):
                errors.append(f"{record.relative}: version must be a positive integer")
            if not example_block(record.body).strip():
                errors.append(
                    f"{record.relative}: no `## Пример` section with a fenced block"
                )
        if record.kind == "decision":
            # `changes` заводят ради перечисления, поэтому пустым он не бывает;
            # `establishes: []` — обычное решение, которое правил не завело.
            if record.props.get("changes") == []:
                errors.append(f"{record.relative}: `changes` must not be empty")
            # Правилом бывает и контракт, и инвариант: инвариант перечисляет имена
            # поверхности не реже, чем контракт, и запрет ссылаться на него оставлял
            # бы такую правку без объявленной причины.
            #
            # `establishes` на неизменяемом решении историчен: позже владельцем того же
            # правила становится другое решение, и обратный ход «правило → нынешний
            # владелец» выше остаётся обязательным. Существовать названное правило
            # обязано всё равно — иначе решение ссылается в пустоту.
            errors.extend(cited_rule_errors(record, "changes", by_id))
            errors.extend(cited_rule_errors(record, "establishes", by_id))

            # Замену объявляют два поля сразу, и поодиночке ни одно её не описывает:
            # `superseded` без преемника — тупик, потому что текст старого решения не
            # правят никогда и спросить, чем его заменили, больше не у кого.
            status = record.props.get("status")
            successor = record.props.get("superseded-by")
            if status in STATUSES and not isinstance(successor, list):
                if status == "superseded" and successor is None:
                    errors.append(
                        f"{record.relative}: `status: superseded` names no successor "
                        f"in `superseded-by`"
                    )
                elif status != "superseded" and successor is not None:
                    errors.append(
                        f"{record.relative}: names a successor in `superseded-by` "
                        f"without `status: superseded`"
                    )

    # Половина замены, записанная с одной стороны, не лучше ненаписанной: `supersedes`
    # ищут от предка к потомку, `superseded-by` — обратно, и разойдясь они дают две
    # разные истории одного решения. Претензия к паре одна, чья бы половина ни молчала.
    for (old_id, new_id), claimant in supersession_claims(found).items():
        old, new = by_id.get(old_id), by_id.get(new_id)
        unknown = next(
            (
                symbol
                for symbol, named in ((old_id, old), (new_id, new))
                if named is None or named.kind != "decision"
            ),
            None,
        )
        if unknown is not None:
            errors.append(
                f"{claimant.relative}: supersession names {unknown}, which is no decision"
            )
        elif old.props.get("superseded-by") != new_id:
            errors.append(f"{old.relative}: does not name {new_id} in `superseded-by`")
        elif not (
            isinstance(new.props.get("supersedes"), list)
            and old_id in new.props["supersedes"]
        ):
            errors.append(f"{new.relative}: does not name {old_id} in `supersedes`")
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
