#!/usr/bin/env python3
"""Замер ключа платформы `/@ <файл>` для задачи #419.

Раннер собирается передавать командную строку `1cv8` файлом через `/@`, чтобы пароль
не стоял в argv процесса. Справка платформы (7.3.11) не говорит, какую кодировку,
конец строки и кавычки она ждёт в файле, когда её запускают не из командного
интерпретатора. Этот скрипт снимает ответы на живой платформе; код раннера под них
пишется после замера.

Скрипт ничего не трогает вне своего каталога: базы, файлы команд и выгрузки
создаются под `--work` (по умолчанию — новый временный каталог) и остаются там для
разбора. Файлы команд случаев E несут пароль открытым текстом: удалите каталог
замера после разбора и не давайте скрипту пароль боевой базы. Каждый случай ограничен `--timeout`: зависание (платформа не поняла файл и
открыла диалог) показывается как `HANG`, процесс снимается.

Запуск:

    python3 scripts/test/probe-command-file.py --v8 "/opt/1cv8/x86_64/8.3.27.1936/1cv8"
    py -3 scripts\\test\\probe-command-file.py --v8 "C:\\Program Files\\1cv8\\8.3.27.1936\\bin\\1cv8.exe"

С базой, где есть пользователь, скрипт проверяет и сами реквизиты (`/N`, `/P`):

    ... --ib "File=C:\\bases\\demo" --user Администратор --password "secret"

Итог печатается таблицей Markdown для `references/1c/confirmed-runtime-measurements.md`.
"""

from __future__ import annotations

import argparse
import dataclasses
import os
import pathlib
import platform
import subprocess
import sys
import tempfile
import time

CYRILLIC = "база"


@dataclasses.dataclass
class Outcome:
    case: str
    question: str
    verdict: str
    detail: str


def encodings_to_try() -> list[str]:
    names = ["utf-8", "utf-8-sig", "utf-16"]
    if os.name == "nt":
        names += ["cp1251", "cp866"]
    return names


def quote(arg: str) -> str:
    """Кавычки так, как их пишет справка платформы: значение с пробелом — в двойных."""
    if arg == "" or any(ch.isspace() for ch in arg):
        return f'"{arg}"'
    return arg


def command_line(args: list[str]) -> str:
    return " ".join(quote(arg) for arg in args)


def write_command_file(path: pathlib.Path, args: list[str], encoding: str, newline: str) -> None:
    path.write_bytes((command_line(args) + newline).encode(encoding))
    if os.name != "nt":
        path.chmod(0o600)


def run(v8: pathlib.Path, argv: list[str], timeout: float) -> tuple[str, float]:
    started = time.monotonic()
    try:
        completed = subprocess.run(
            [str(v8), *argv],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=timeout,
            check=False,
        )
    except subprocess.TimeoutExpired:
        return "HANG", time.monotonic() - started
    return f"exit {completed.returncode}", time.monotonic() - started


def read_log(path: pathlib.Path) -> str:
    if not path.exists():
        return "нет /Out"
    raw = path.read_bytes()
    for encoding in ("utf-8-sig", "cp1251"):
        try:
            text = raw.decode(encoding)
            break
        except UnicodeDecodeError:
            continue
    else:
        text = raw.decode("utf-8", "replace")
    text = " ".join(text.split())
    return text[:160] or "пустой /Out"


def file_ib(path: pathlib.Path) -> str:
    return f"File='{path.as_posix() if os.name != 'nt' else str(path)}'"


def created(ib_dir: pathlib.Path) -> bool:
    return (ib_dir / "1Cv8.1CD").exists()


def siblings(root: pathlib.Path) -> str:
    names = sorted(p.name for p in root.iterdir() if p.is_dir())
    return ", ".join(names) or "—"


class Probe:
    def __init__(self, v8: pathlib.Path, work: pathlib.Path, timeout: float) -> None:
        self.v8 = v8
        self.work = work
        self.timeout = timeout
        self.outcomes: list[Outcome] = []
        self.counter = 0

    def note(self, case: str, question: str, verdict: str, detail: str) -> None:
        self.outcomes.append(Outcome(case, question, verdict, detail))
        print(f"[{verdict}] {case}: {detail}", flush=True)

    def command_file(self, args: list[str], encoding: str = "utf-8", newline: str = "") -> pathlib.Path:
        self.counter += 1
        path = self.work / "cmd" / f"{self.counter:02d}.txt"
        path.parent.mkdir(parents=True, exist_ok=True)
        write_command_file(path, args, encoding, newline)
        return path

    def create_case(self, case: str, question: str, root: pathlib.Path, name: str,
                    *, via_file: bool, encoding: str = "utf-8", newline: str = "") -> None:
        root.mkdir(parents=True, exist_ok=True)
        ib_dir = root / name
        args = ["CREATEINFOBASE", file_ib(ib_dir)]
        if via_file:
            try:
                path = self.command_file(args, encoding, newline)
            except UnicodeEncodeError:
                self.note(case, question, "SKIP", f"{encoding} не кодирует путь")
                return
            status, elapsed = run(self.v8, ["/@", str(path)], self.timeout)
        else:
            status, elapsed = run(self.v8, args, self.timeout)
        ok = created(ib_dir)
        verdict = "OK" if ok and status == "exit 0" else ("HANG" if status == "HANG" else "FAIL")
        self.note(case, question, verdict,
                  f"{status}, {elapsed:.1f} с; каталоги рядом: {siblings(root)}")

    def designer_case(self, case: str, question: str, ib: str, extra: list[str],
                      *, via_file: bool, expect_success: bool, dump_name: str = "dump",
                      trailing_separator: bool = False, encoding: str = "utf-8") -> None:
        self.counter += 1
        out_dir = self.work / "designer" / f"{self.counter:02d}"
        out_dir.mkdir(parents=True, exist_ok=True)
        dump = out_dir / dump_name
        log = out_dir / "out.log"
        args = ["DESIGNER", "/DisableStartupDialogs", "/DisableStartupMessages",
                *connection_args(ib), *extra,
                "/Out", str(log), "/DumpConfigToFiles", str(dump) + (os.sep if trailing_separator else "")]
        if via_file:
            try:
                path = self.command_file(args, encoding)
            except UnicodeEncodeError:
                self.note(case, question, "SKIP", f"{encoding} не кодирует аргументы")
                return
            status, elapsed = run(self.v8, ["/@", str(path)], self.timeout)
        else:
            status, elapsed = run(self.v8, args, self.timeout)
        dumped = (dump / "Configuration.xml").exists()
        if status == "HANG":
            verdict = "HANG"
        elif expect_success:
            verdict = "OK" if dumped and status == "exit 0" else "FAIL"
        else:
            verdict = "OK" if not dumped and status != "exit 0" else "FAIL"
        self.note(case, question, verdict,
                  f"{status}, {elapsed:.1f} с, выгрузка {'есть' if dumped else 'нет'}; {read_log(log)}")


def connection_args(ib: str) -> list[str]:
    if ib.lower().startswith("file="):
        return ["/F", ib.split("=", 1)[1].strip().strip("'\"")]
    return ["/IBConnectionString", ib]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--v8", required=True, type=pathlib.Path, help="путь к 1cv8 (не к 1cestart)")
    parser.add_argument("--work", type=pathlib.Path, help="каталог замера; по умолчанию новый временный")
    parser.add_argument("--timeout", type=float, default=180.0, help="предел одного случая, с")
    parser.add_argument("--ib", help="база с пользователем: File=... или Srvr=...;Ref=...")
    parser.add_argument("--user", help="пользователь этой базы")
    parser.add_argument("--password", help="его пароль")
    options = parser.parse_args()

    work = options.work or pathlib.Path(tempfile.mkdtemp(prefix="v8-at-probe-"))
    work.mkdir(parents=True, exist_ok=True)
    probe = Probe(options.v8, work, options.timeout)
    print(f"Каталог замера: {work}", flush=True)

    # 1. Работает ли `/@` вообще и какой конец строки он терпит.
    probe.create_case("A1", "argv, эталон", work / "a", "argv", via_file=False)
    for label, newline in (("без перевода строки", ""), ("LF", "\n"), ("CRLF", "\r\n")):
        probe.create_case(f"A2 {label}", "`/@` понят, конец строки", work / "a",
                          f"at-{len(newline)}", via_file=True, newline=newline)

    # 2. Значение с пробелом в двойных кавычках.
    probe.create_case("B1", "`/@`, путь с пробелом в кавычках", work / "b", "with space", via_file=True)

    # 3. Кодировка файла: какой из вариантов создал каталог с верным кириллическим именем.
    for encoding in encodings_to_try():
        root = work / "c" / encoding
        probe.create_case(f"C {encoding}", "кодировка файла, кириллица в пути", root, CYRILLIC,
                          via_file=True, encoding=encoding)

    # 4. Пакетный Конфигуратор через `/@` на базе без пользователей.
    base = work / "a" / "argv"
    if created(base):
        ib = f"File={base}"
        probe.designer_case("D1", "argv, эталон Конфигуратора", ib, [], via_file=False, expect_success=True)
        probe.designer_case("D2", "`/@`, пакетный Конфигуратор", ib, [], via_file=True, expect_success=True)
        probe.designer_case("D3", "`/@`, /N /P на базе без пользователей", ib,
                            ["/N", "Admin", "/P", "secret"], via_file=True, expect_success=True)
        probe.designer_case("D4", "argv, /N /P на базе без пользователей", ib,
                            ["/N", "Admin", "/P", "secret"], via_file=False, expect_success=True)
        if os.name == "nt":
            probe.designer_case("D5", "`/@`, путь с пробелом и обратной косой в конце", ib, [],
                                via_file=True, expect_success=True, dump_name="dump dir",
                                trailing_separator=True)
    else:
        probe.note("D", "Конфигуратор", "SKIP", "эталонная база A1 не создана")

    # 5. Настоящие реквизиты: верный и неверный пароль. Пароль с двойной кавычкой
    # скрипт пишет как есть: правило кавычек внутри файла справка не называет.
    if options.ib and options.user is not None:
        password = options.password or ""
        creds = ["/N", options.user] + (["/P", password] if password else [])
        wrong = ["/N", options.user, "/P", password + "-wrong"]
        probe.designer_case("E1", "argv, верные реквизиты", options.ib, creds,
                            via_file=False, expect_success=True)
        for encoding in encodings_to_try():
            probe.designer_case(f"E2 {encoding}", "`/@`, верные реквизиты", options.ib, creds,
                                via_file=True, expect_success=True, encoding=encoding)
        probe.designer_case("E3", "`/@`, неверный пароль: отказ, а не зависание", options.ib, wrong,
                            via_file=True, expect_success=False)
    else:
        probe.note("E", "реквизиты", "SKIP", "не задана база с пользователем (--ib/--user/--password)")

    print()
    print(f"Платформа: `{options.v8}`; ОС: {platform.platform()}; Python {platform.python_version()}")
    print()
    print("| Случай | Вопрос | Итог | Подробности |")
    print("| --- | --- | --- | --- |")
    for item in probe.outcomes:
        detail = item.detail.replace("|", "\\|")
        print(f"| {item.case} | {item.question} | {item.verdict} | {detail} |")
    return 0


if __name__ == "__main__":
    sys.exit(main())
