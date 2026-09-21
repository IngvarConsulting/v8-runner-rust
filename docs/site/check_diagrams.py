# -*- coding: utf-8 -*-
"""Проверка схем из diagrams.py: подпись ребра не должна ложиться на границу рамки, на заголовок
рамки, на узел или на другую подпись; узел не должен пересекать границу рамки, в которую не входит.
Порог — глубина наложения больше 5 px, касания не считаются.

Запуск: python3 docs/site/check_diagrams.py — выводит замечания и завершается с кодом 1, если они есть.
"""
import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import diagrams
from diagrams import text_w

THRESHOLD = 5

def boxes(d):
    d.render()  # заполняет f['box']
    labels = []
    for e in d.edges:
        if not e['label']: continue
        path, (lx, ly) = d.edge_geometry(e)
        lx += e['dx']; ly += e['dy']
        lines = e['label'].split('\n')
        w = int(text_w(e['label'], 12.5)) + 14; h = 8 + 16 * len(lines)
        labels.append((lx - w / 2, ly - h / 2, lx + w / 2, ly + h / 2, e['label']))
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

def check():
    problems = []
    for id, d in diagrams.DIAGRAMS.items():
        labels, nodes, frames = boxes(d)
        for L in labels:
            for (fr, fid, members, title) in frames:
                v = border_depth(L, fr)
                if v > THRESHOLD: problems.append((id, 'подпись на границе рамки', L[4], fid, v))
                v = depth(L, title_box(fr, title))
                if v > THRESHOLD: problems.append((id, 'подпись на заголовке рамки', L[4], fid, v))
            for N in nodes:
                v = depth(L, N)
                if v > THRESHOLD: problems.append((id, 'подпись на узле', L[4], N[4], v))
            for M in labels:
                if M is not L and M[4] < L[4]:
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
