# -*- coding: utf-8 -*-
"""Проверка схем из diagrams.py: подпись ребра не должна ложиться на границу рамки, на заголовок
рамки, на узел или на другую подпись; узел не должен пересекать границу рамки, в которую не входит.
Порог — глубина наложения больше 5 px, касания не считаются.

Запуск: python3 docs/site/check_diagrams.py — выводит замечания и завершается с кодом 1, если они есть.
"""
import math, os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import diagrams
from diagrams import text_w

THRESHOLD = 5

def boxes(d):
    d.render()  # заполняет f['box']
    labels = []
    for index, e in enumerate(d.edges):
        if not e['label']: continue
        path, (lx, ly) = d.edge_geometry(e)
        lx += e['dx']; ly += e['dy']
        lines = e['label'].split('\n')
        w = int(text_w(e['label'], 12.5)) + 14; h = 8 + 16 * len(lines)
        labels.append((lx - w / 2, ly - h / 2, lx + w / 2, ly + h / 2, e['label'], index))
    nodes = [(n['x'] - n['w'] / 2, n['y'] - n['h'] / 2, n['x'] + n['w'] / 2, n['y'] + n['h'] / 2, n['id']) for n in d.nodes.values()]
    frames = [(f['box'], f['id'], set(f['members']), f['title']) for f in d.frames]
    return labels, nodes, frames

def depth(a, b):
    """глубина наложения двух коробок: min по осям, ≤ 0 — не пересекаются"""
    return min(min(a[2], b[2]) - max(a[0], b[0]), min(a[3], b[3]) - max(a[1], b[1]))

def border_depth(box, frame):
    """насколько коробка заходит на границу рамки: min(часть внутри, часть снаружи)"""
    x0, y0, x1, y1 = frame
    if depth(box, frame) <= 0: return 0
    if min(box[0] - x0, x1 - box[2], box[1] - y0, y1 - box[3]) >= 0: return 0
    outside = max(x0 - box[0], box[2] - x1, y0 - box[1], box[3] - y1)
    return min(depth(box, frame), outside)

def title_box(frame, title):
    x0, y0, x1, y1 = frame
    return (x0 + 14, y0 + 5, x0 + 14 + len(title) * 7.2, y0 + 22)

def check_routes(d):
    """Прямые выходы/входы по нормали ≥30 px; касательные стыков отличаются ≤1°."""
    problems, ports, routes = [], {}, {}
    for index, e in enumerate(d.edges):
        if not e['arrow']: continue
        path, _ = d.edge_geometry(e)
        segments = [(cmd, p) for cmd, p in d.path_segments(path) if cmd != 'M']
        points = d.path_points(path)
        x, y, normal = d.anchor(e['b'], e['fb'])
        end, before = points[-1], points[-2]
        dx, dy = before[0] - end[0], before[1] - end[1]
        length = dx * normal[0] + dy * normal[1]
        error = max(abs(end[0] - x), abs(end[1] - y), abs(dx * normal[1] - dy * normal[0]))
        name = e['a'] + ' → ' + e['b']
        # У капсулы углы bounding box лежат вне нарисованного контура.
        # Осевые нормали anchor() допустимы только на прямых участках формы.
        for nid, spec in ((e['a'], e['fa']), (e['b'], e['fb'])):
            node = d.nodes[nid]
            if node['kind'] not in ('svc', 'cmd'): continue
            px, py, _ = d.anchor(nid, spec)
            side = spec.split(':')[0]
            on_outline = (abs(py - node['y']) <= 0.1 if side in ('l', 'r') else
                          abs(px - node['x']) <= (node['w'] - node['h']) / 2 + 0.1)
            if not on_outline:
                problems.append((d.id, 'порт вне прямого контура капсулы', name, nid + ':' + spec, 1))
        sx, sy, source_normal = d.anchor(e['a'], e['fa'])
        start_error = max(abs(points[0][0] - sx), abs(points[0][1] - sy))
        if start_error > 0.1:
            problems.append((d.id, 'начало вне объявленного порта', name, e['fa'], start_error))
        first_command, departure = segments[0]
        vx, vy = departure[-1][0] - sx, departure[-1][1] - sy
        exit_length = vx * source_normal[0] + vy * source_normal[1]
        exit_error = abs(vx * source_normal[1] - vy * source_normal[0])
        if first_command != 'L' or exit_error > 0.1 or exit_length < 30:
            problems.append((d.id, 'наклонный или короткий выход', name, e['fa'], max(exit_error, 30 - exit_length)))
        last_command = segments[-1][0]
        if last_command != 'L' or error > 0.1 or length < 30:
            problems.append((d.id, 'наклонный или короткий вход', name, e['fb'], max(error, 30 - length)))
        tangents = []
        for cmd, p in segments:
            start = (p[1][0] - p[0][0], p[1][1] - p[0][1])
            end = (p[-1][0] - p[-2][0], p[-1][1] - p[-2][1])
            if min(math.hypot(*start), math.hypot(*end)) < 1e-6:
                problems.append((d.id, 'вырожденная касательная', name, cmd, 1))
            if cmd == 'C':
                axis = (p[-1][0] - p[0][0], p[-1][1] - p[0][1])
                offsets = [(q[0] - p[0][0], q[1] - p[0][1]) for q in p]
                if all(abs(v[0] * axis[1] - v[1] * axis[0]) < 0.1 for v in offsets):
                    positions = [v[0] * axis[0] + v[1] * axis[1] for v in offsets]
                    if any(b < a - 0.1 for a, b in zip(positions, positions[1:])):
                        problems.append((d.id, 'обратный ход прямой кривой', name, cmd, 1))
            tangents.append((start, end))
        for (_, incoming), (outgoing, _) in zip(tangents, tangents[1:]):
            cross = incoming[0] * outgoing[1] - incoming[1] * outgoing[0]
            dot = incoming[0] * outgoing[0] + incoming[1] * outgoing[1]
            angle = abs(math.degrees(math.atan2(cross, dot)))
            if angle > 1:
                problems.append((d.id, 'излом на стыке сегментов', name, 'касательные', angle))
        side = e['fb'].split(':')[0]
        n = d.nodes[e['b']]
        along = y if side in ('l', 'r') else x
        low = n['y'] - n['h'] / 2 if side in ('l', 'r') else n['x'] - n['w'] / 2
        high = n['y'] + n['h'] / 2 if side in ('l', 'r') else n['x'] + n['w'] / 2
        if n['kind'] in ('doc', 'docoff') and side in ('l', 'r'): high -= 8
        clearance = min(along - low, high - along)
        if clearance < 8:
            problems.append((d.id, 'вход близко к углу или волне', name, e['b'], 8 - clearance))
        key = (e['b'], side)
        for previous in ports.get(key, []):
            gap = abs(along - previous)
            if gap < 18:
                problems.append((d.id, 'точки входа слиплись', name, e['b'], 18 - gap))
        ports.setdefault(key, []).append(along)
        # Проверяем также прямые участки между точками, а не только концы кривых.
        samples = []
        for a, b in zip(points, points[1:]):
            steps = max(1, math.ceil(math.dist(a, b) / 2))
            samples.extend((a[0] + (b[0] - a[0]) * i / steps, a[1] + (b[1] - a[1]) * i / steps) for i in range(steps + 1))
        routes[index] = samples
        for nid, node in d.nodes.items():
            if any(abs(px - node['x']) < node['w'] / 2 - 2 and abs(py - node['y']) < node['h'] / 2 - 2 for px, py in samples):
                problems.append((d.id, 'ребро проходит через узел', name, nid, 6))
    labels, _, _ = boxes(d)
    for x0, y0, x1, y1, label, owner in labels:
        def inside(p, margin=0):
            return x0 + margin < p[0] < x1 - margin and y0 + margin < p[1] < y1 - margin
        e = d.edges[owner]
        points = d.path_points(d.edge_geometry(e)[0])
        gap = min(math.hypot(max(x0 - x, 0, x - x1), max(y0 - y, 0, y - y1))
                  for x, y in (points[0], points[-1]))
        if gap < 10 - 0.1:
            problems.append((d.id, 'подпись закрывает начало или наконечник', label, e['a'] + ' → ' + e['b'], 10 - gap))
        if not any(inside(p) for p in routes.get(owner, [])):
            problems.append((d.id, 'подпись оторвана от своего ребра', label, str(owner), 1))
        for index, points in routes.items():
            if index != owner and any(inside(p, 2) for p in points):
                e = d.edges[index]
                problems.append((d.id, 'чужое ребро под подписью', label, e['a'] + ' → ' + e['b'], 1))
    return problems

def check():
    problems = []
    for id, d in diagrams.DIAGRAMS.items():
        labels, nodes, frames = boxes(d)
        if id in ('map', 'd-docker-cluster', 'd-file-web', 'd-cluster-web', 'd-web-remote', 'd-web-dual', 'd-web-two-infobases'): problems.extend(check_routes(d))
        for n in d.nodes.values():
            if n['kind'] in ('doc', 'docoff'):
                # Верх волны: h/2 - 8 - 6; нижняя строка: центр + 9*(строки-1) + 7.
                gap = n['h'] / 2 - 14 - (9 * (len(n['text'].split('\n')) - 1) + 7)
                if gap < 8:
                    problems.append((id, 'текст близко к волнистому краю', n['text'], n['id'], 8 - gap))
        for i, L in enumerate(labels):
            for (fr, fid, members, title) in frames:
                v = border_depth(L, fr)
                if v > THRESHOLD: problems.append((id, 'подпись на границе рамки', L[4], fid, v))
                v = depth(L, title_box(fr, title))
                if v > THRESHOLD: problems.append((id, 'подпись на заголовке рамки', L[4], fid, v))
            for N in nodes:
                v = depth(L, N)
                if v > THRESHOLD: problems.append((id, 'подпись на узле', L[4], N[4], v))
            for M in labels[:i]:  # каждая пара один раз, одинаковый текст не повод пропускать
                v = depth(L, M)
                if v > THRESHOLD: problems.append((id, 'подписи друг на друге', L[4], M[4], v))
        for N in nodes:
            for (fr, fid, members, title) in frames:
                if N[4] in members: continue
                v = border_depth(N, fr)
                if v > THRESHOLD: problems.append((id, 'узел на границе рамки', N[4], fid, v))
    return problems

if __name__ == '__main__':
    problems = check()
    for p in problems: print('%-22s %-28s %-40s %-14s %.0f px' % p)
    print(len(problems), 'замечаний')
    sys.exit(1 if problems else 0)
