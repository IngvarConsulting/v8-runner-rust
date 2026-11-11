"""Medians of result rows and a runner trace with offsets."""
import json
import re
import statistics

from describe import EXECUTORS

TIME = re.compile(r"^(\d\d):(\d\d):(\d\d(?:\.\d+)?)\s")


def rows(files):
    for path in files:
        with open(path, encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if line:
                    yield json.loads(line)


def fmt(value):
    return f"{value:.1f}".replace(".", ",")


def table(files, steps=False):
    """Markdown table: a row per stand and scenario, a column per executor, `median (n)` seconds.

    Counted: measured rows that succeeded and whose check did not fail, warm-ups excluded.
    Failed and unverified runs are counted separately under the table.
    """
    cells, step_cells, skipped, titles = {}, {}, [], {}
    for row in rows(files):
        if row.get("event") != "measure" or row.get("warmup"):
            continue
        key = (row.get("stand"), row.get("scenario"))
        titles.setdefault(key, row.get("files"))
        if not row.get("ok") or row.get("verified") is False:
            skipped.append(f"{row.get('stand')} {row.get('scenario')} {row.get('executor')} #{row.get('rep')}: "
                           f"{'not verified' if row.get('ok') else row.get('error')}")
            continue
        cells.setdefault(key, {}).setdefault(row["executor"], []).append(row["seconds"])
        for step, seconds in (row.get("steps") or {}).items():
            step_cells.setdefault(key, {}).setdefault(row["executor"], {}).setdefault(step, []).append(seconds)
    used = [e for e in EXECUTORS if any(e in by for by in cells.values())]
    lines = ["| Стенд | Сценарий | " + " | ".join(used) + " |", "|---|---|" + "---:|" * len(used)]
    for key in sorted(cells):
        by = cells[key]
        out = []
        for e in used:
            values = by.get(e)
            out.append(f"{fmt(statistics.median(values))} ({len(values)})" if values else "—")
        lines.append(f"| {key[0]} | {key[1]} | " + " | ".join(out) + " |")
    if steps and step_cells:
        lines += ["", "| Стенд | Сценарий | Исполнитель | Шаги, медиана |", "|---|---|---|---|"]
        for key in sorted(step_cells):
            for e in used:
                parts = step_cells[key].get(e)
                if parts:
                    text = ", ".join(f"{s} {fmt(statistics.median(v))}" for s, v in parts.items())
                    lines.append(f"| {key[0]} | {key[1]} | {e} | {text} |")
    if skipped:
        lines += ["", "Не вошли в медианы:"] + [f"- {s}" for s in skipped]
    return "\n".join(lines)


def seconds_of(line):
    m = TIME.match(line)
    if not m:
        return None
    h, mnt, s = m.groups()
    return int(h) * 3600 + int(mnt) * 60 + float(s)


def phases(trace):
    """Each trace line with its offset from the first line and the gap after the previous one."""
    out, first, prev = [], None, None
    with open(trace, encoding="utf-8", errors="replace") as f:
        for line in f:
            t = seconds_of(line)
            if t is None:
                continue
            first = t if first is None else first
            gap = 0.0 if prev is None else t - prev
            prev = t
            out.append(f"{t - first:8.2f} +{gap:6.2f}  {line[13:].strip()[:180]}")
    return "\n".join(out)
