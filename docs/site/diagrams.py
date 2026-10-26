# -*- coding: utf-8 -*-
"""Схемы сайта: рисуются встроенным SVG из описаний ниже и вписываются в страницы
между маркерами <!-- diagram:ид --> … <!-- /diagram:ид -->.

Запуск: python3 docs/site/diagrams.py            — перерисовать схемы во всех страницах
        python3 docs/site/diagrams.py --print ид — вывести SVG одной схемы
"""
import html, re, sys, os

CHAR = {14: 7.6, 13: 7.0, 12.5: 6.7}

def text_w(text, size):
    return max(len(line) for line in text.split('\n')) * CHAR[size]

class Diagram:
    def __init__(self, id, title):
        self.id, self.title = id, title
        self.nodes, self.frames, self.edges, self.notes = {}, [], [], []

    # --- элементы ---
    def node(self, id, text, kind, x, y, w=None, h=None):
        lines = text.split('\n')
        w = w or max(96, int(text_w(text, 14)) + (48 if kind in ('svc', 'cmd') else 28))
        h = h or (44 if len(lines) == 1 else 22 + 18 * len(lines))
        # Волнистый край занимает нижнюю часть документа; оставляем зазор под текстом.
        if kind in ('doc', 'docoff'):
            h += 18
        self.nodes[id] = dict(id=id, text=text, kind=kind, x=x, y=y, w=w, h=h)
        return id

    def frame(self, id, title, members, kind='host', pad=18, top=26, extra=()):
        self.frames.append(dict(id=id, title=title, members=members, kind=kind, pad=pad, top=top, extra=extra))

    def edge(self, a, b, label=None, dashed=False, fa='r', fb='l', t=0.5, dx=0, dy=0, path=None, arrow=True, lx=None, ly=None, entry_stub=0, exit_stub=0):
        self.edges.append(dict(a=a, b=b, label=label, dashed=dashed, fa=fa, fb=fb, t=t, dx=dx, dy=dy, path=path, arrow=arrow, lx=lx, ly=ly, entry_stub=entry_stub, exit_stub=exit_stub))

    def note(self, x, y, text, size=12.5):
        self.notes.append(dict(x=x, y=y, text=text, size=size))

    # --- геометрия ---
    def anchor(self, node, spec):
        n = self.nodes[node]
        side, _, frac = spec.partition(':')
        f = float(frac) if frac else 0.5
        if side == 'l': return (n['x'] - n['w'] / 2, n['y'] - n['h'] / 2 + n['h'] * f, (-1, 0))
        if side == 'r': return (n['x'] + n['w'] / 2, n['y'] - n['h'] / 2 + n['h'] * f, (1, 0))
        if side == 't': return (n['x'] - n['w'] / 2 + n['w'] * f, n['y'] - n['h'] / 2, (0, -1))
        if side == 'b': return (n['x'] - n['w'] / 2 + n['w'] * f, n['y'] + n['h'] / 2, (0, 1))
        raise ValueError(spec)

    @staticmethod
    def path_segments(path):
        """Разбирает authored M/L/C/Q: команда и её точки, включая начало сегмента."""
        segments, cur = [], None
        for cmd, args in re.findall(r'([MLCQ])\s*([^MLCQ]*)', path):
            v = [float(x) for x in re.findall(r'-?[\d.]+', args)]
            stride = {'M': 2, 'L': 2, 'C': 6, 'Q': 4}[cmd]
            for i in range(0, len(v), stride):
                p = [(v[j], v[j + 1]) for j in range(i, i + stride, 2)]
                segments.append((cmd, p if cmd == 'M' else [cur] + p)); cur = p[-1]
        return segments

    @staticmethod
    def path_points(path):
        """точки пути с шагом по кривым — чтобы рамка картинки учитывала линии, а не только узлы"""
        pts = []
        for cmd, p in Diagram.path_segments(path):
            if cmd in ('M', 'L'): pts.append(p[-1])
            elif cmd == 'C': pts += [Diagram.bezier(*p, k / 10) for k in range(11)]
            elif cmd == 'Q':
                start, c, e = p
                pts += [((1 - t) ** 2 * start[0] + 2 * (1 - t) * t * c[0] + t * t * e[0], (1 - t) ** 2 * start[1] + 2 * (1 - t) * t * c[1] + t * t * e[1]) for t in [k / 10 for k in range(11)]]
        return pts

    @staticmethod
    def rounded_path(points, radius=18):
        """Скругляет углы заданной ломаной; положение маршрута задаёт автор схемы."""
        clean = [points[0]]
        for p in points[1:]:
            if p != clean[-1]: clean.append(p)
        path = 'M%.1f,%.1f' % clean[0]
        current = clean[0]
        for a, b, c in zip(clean, clean[1:], clean[2:]):
            incoming, outgoing = (b[0] - a[0], b[1] - a[1]), (c[0] - b[0], c[1] - b[1])
            if incoming[0] * outgoing[1] == incoming[1] * outgoing[0]:
                continue
            la, lc = (incoming[0] ** 2 + incoming[1] ** 2) ** 0.5, (outgoing[0] ** 2 + outgoing[1] ** 2) ** 0.5
            r = min(radius, la / 2, lc / 2)
            before = (b[0] - incoming[0] / la * r, b[1] - incoming[1] / la * r)
            after = (b[0] + outgoing[0] / lc * r, b[1] + outgoing[1] / lc * r)
            if before != current: path += ' L%.1f,%.1f' % before
            path += ' Q%.1f,%.1f %.1f,%.1f' % (*b, *after)
            current = after
        return path + (' L%.1f,%.1f' % clean[-1] if clean[-1] != current else '')

    @staticmethod
    def bezier(p0, p1, p2, p3, t):
        u = 1 - t
        return (u*u*u*p0[0] + 3*u*u*t*p1[0] + 3*u*t*t*p2[0] + t*t*t*p3[0],
                u*u*u*p0[1] + 3*u*u*t*p1[1] + 3*u*t*t*p2[1] + t*t*t*p3[1])

    @staticmethod
    def at_x(p, x):
        """точка кривой p (4 точки Безье) с абсциссой x: подпись ложится на линию"""
        lo, hi = 0.0, 1.0
        asc = p[3][0] >= p[0][0]
        for _ in range(40):
            mid = (lo + hi) / 2
            if (Diagram.bezier(*p, mid)[0] < x) == asc: lo = mid
            else: hi = mid
        return Diagram.bezier(*p, (lo + hi) / 2)

    def edge_geometry(self, e):
        if e['lx'] is not None and e['ly'] is not None and e['path']:
            return e['path'], (e['lx'], e['ly'])
        if e['path']:
            segments = self.path_segments(e['path'])
            p = next((p for cmd, p in segments if cmd == 'C'), None)
            if p:
                if e['lx'] is not None: return e['path'], self.at_x(p, e['lx'])
                return e['path'], self.bezier(*p, e['t'])
            start, end = next(p for cmd, p in segments if cmd == 'L')
            return e['path'], (start[0] + (end[0] - start[0]) * e['t'], start[1] + (end[1] - start[1]) * e['t'])
        (x1, y1, d1), (x2, y2, d2) = self.anchor(e['a'], e['fa']), self.anchor(e['b'], e['fb'])
        dist = max(abs(x2 - x1), abs(y2 - y1))
        k = max(40, dist * 0.45)
        departure = (x1 + d1[0] * e['exit_stub'], y1 + d1[1] * e['exit_stub'])
        stub = e['entry_stub']
        entry = (x2 + d2[0] * stub, y2 + d2[1] * stub)
        if (stub or e['exit_stub']) and d1 == (-d2[0], -d2[1]):
            # Между близкими блоками длинные handles разворачивают кривую назад.
            available = (entry[0] - departure[0]) * d1[0] + (entry[1] - departure[1]) * d1[1]
            if available > 0: k = min(k, available / 2)
        c1 = (departure[0] + d1[0] * k, departure[1] + d1[1] * k)
        c2 = (entry[0] + d2[0] * k, entry[1] + d2[1] * k)
        path = 'M%.1f,%.1f' % (x1, y1)
        if e['exit_stub']: path += ' L%.1f,%.1f' % departure
        path += ' C%.1f,%.1f %.1f,%.1f %.1f,%.1f' % (c1[0], c1[1], c2[0], c2[1], entry[0], entry[1])
        if stub: path += ' L%.1f,%.1f' % (x2, y2)
        if e['lx'] is not None and e['ly'] is not None: return path, (e['lx'], e['ly'])
        if e['lx'] is not None: return path, self.at_x([departure, c1, c2, entry], e['lx'])
        return path, self.bezier(departure, c1, c2, entry, e['t'])

    # --- отрисовка ---
    def render(self):
        out = []
        bbox = [1e9, 1e9, -1e9, -1e9]
        def grow(x0, y0, x1, y1):
            bbox[0] = min(bbox[0], x0); bbox[1] = min(bbox[1], y0); bbox[2] = max(bbox[2], x1); bbox[3] = max(bbox[3], y1)
        # рамки — снизу
        for f in self.frames:
            xs, ys = [], []
            for m in f['members']:
                n = self.nodes[m]; xs += [n['x'] - n['w'] / 2, n['x'] + n['w'] / 2]; ys += [n['y'] - n['h'] / 2, n['y'] + n['h'] / 2]
            for (ex0, ey0, ex1, ey1) in f['extra']: xs += [ex0, ex1]; ys += [ey0, ey1]
            x0, y0, x1, y1 = min(xs) - f['pad'], min(ys) - f['pad'] - f['top'], max(xs) + f['pad'], max(ys) + f['pad']
            f['box'] = (x0, y0, x1, y1)
        for f in sorted(self.frames, key=lambda f: -((f['box'][2] - f['box'][0]) * (f['box'][3] - f['box'][1]))):
            x0, y0, x1, y1 = f['box']; grow(x0, y0, x1, y1)
            out.append('<g class="fr fr-%s"><rect x="%.1f" y="%.1f" width="%.1f" height="%.1f" rx="12"/><text x="%.1f" y="%.1f">%s</text></g>'
                       % (f['kind'], x0, y0, x1 - x0, y1 - y0, x0 + 14, y0 + 18, html.escape(f['title'])))
        # рёбра
        labels = []
        for e in self.edges:
            path, (lx, ly) = self.edge_geometry(e)
            cls = 'e' + (' dash' if e['dashed'] else '') + ('' if e['arrow'] else ' noarr')
            out.append('<path class="%s" d="%s"/>' % (cls, path))
            for (px, py) in self.path_points(path): grow(px - 4, py - 4, px + 4, py + 4)
            if e['label']:
                labels.append((lx + e['dx'], ly + e['dy'], e['label']))
        # узлы
        for n in self.nodes.values():
            x0, y0, w, h = n['x'] - n['w'] / 2, n['y'] - n['h'] / 2, n['w'], n['h']
            grow(x0, y0, x0 + w, y0 + h)
            k = n['kind']
            if k in ('doc', 'docoff'):
                shape = ('<path d="M%.1f,%.1f h%.1f v%.1f q%.1f,%.1f %.1f,0 q%.1f,%.1f %.1f,0 z"/>'
                         % (x0, y0, w, h - 8, -w / 4, 12, -w / 2, -w / 4, -12, -w / 2))
            elif k == 'cyl':
                ry = 7
                shape = ('<path d="M%.1f,%.1f v%.1f a%.1f,%.1f 0 0 0 %.1f,0 v%.1f"/><ellipse cx="%.1f" cy="%.1f" rx="%.1f" ry="%.1f"/>'
                         % (x0, y0 + ry, h - 2 * ry, w / 2, ry, w, -(h - 2 * ry), x0 + w / 2, y0 + ry, w / 2, ry))
            elif k in ('svc', 'cmd'):
                shape = '<rect x="%.1f" y="%.1f" width="%.1f" height="%.1f" rx="%.1f"/>' % (x0, y0, w, h, h / 2)
            else:
                shape = '<rect x="%.1f" y="%.1f" width="%.1f" height="%.1f" rx="8"/>' % (x0, y0, w, h)
            lines = n['text'].split('\n')
            ty = n['y'] - (len(lines) - 1) * 9
            text = ''.join('<tspan x="%.1f" y="%.1f">%s</tspan>' % (n['x'], ty + i * 18, html.escape(l)) for i, l in enumerate(lines))
            out.append('<g class="n n-%s">%s<text>%s</text></g>' % (k, shape, text))
        # подписи рёбер — поверх
        for (lx, ly, label) in labels:
            lines = label.split('\n')
            w = int(text_w(label, 12.5)) + 14; h = 8 + 16 * len(lines)
            grow(lx - w / 2, ly - h / 2, lx + w / 2, ly + h / 2)
            ty = ly - (len(lines) - 1) * 8
            text = ''.join('<tspan x="%.1f" y="%.1f">%s</tspan>' % (lx, ty + i * 16, html.escape(l)) for i, l in enumerate(lines))
            out.append('<g class="lbl"><rect x="%.1f" y="%.1f" width="%d" height="%d" rx="4"/><text>%s</text></g>' % (lx - w / 2, ly - h / 2, w, h, text))
        for nt in self.notes:
            out.append('<text class="note" x="%.1f" y="%.1f">%s</text>' % (nt['x'], nt['y'], html.escape(nt['text'])))
            grow(nt['x'], nt['y'] - 14, nt['x'] + text_w(nt['text'], 12.5), nt['y'] + 4)
        m = 10
        vb = (bbox[0] - m, bbox[1] - m, bbox[2] - bbox[0] + 2 * m, bbox[3] - bbox[1] + 2 * m)
        svg = ('<svg viewBox="%.1f %.1f %.1f %.1f" xmlns="http://www.w3.org/2000/svg" style="max-width:%dpx">\n'
               '<defs><marker id="arr-%s" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" markerHeight="8" orient="auto"><path d="M0,0 L10,5 L0,10 z"/></marker></defs>\n'
               % (vb[0], vb[1], vb[2], vb[3], int(vb[2]), self.id))
        body = '\n'.join(out).replace('class="e', 'marker-end="url(#arr-%s)" class="e' % self.id)
        body = body.replace('marker-end="url(#arr-%s)" class="e noarr"' % self.id, 'class="e noarr"').replace('marker-end="url(#arr-%s)" class="e dash noarr"' % self.id, 'class="e dash noarr"')
        return ('<div class="diagram" role="img" aria-label="%s">\n%s%s\n</svg>\n</div>' % (html.escape(self.title), svg, body))

# ============================ схемы ============================
DIAGRAMS = {}
def diagram(id, title):
    d = Diagram(id, title); DIAGRAMS[id] = d; return d

def host_fan(d, targets):
    """раннер слева и веер рёбер к утилитам: targets = [(id, label, fa, t)];
    подпись стоит посередине зазора между раннером и утилитой, на своей линии"""
    r = d.nodes['R']
    for tid, label, fa, t, *rest in targets:
        n = d.nodes[tid]
        d.edge('R', tid, label, fa=fa, fb=rest[0] if rest else 'l', lx=(r['x'] + r['w'] / 2 + n['x'] - n['w'] / 2) / 2)

def general_map(id):
    d = diagram(id, 'Общая карта: XML и EDT, утилиты и общие хранилища базы в разных режимах')
    d.node('R', 'v8-runner\nCLI', 'run', 60, 570, w=140)
    d.node('EDT', '1cedtcli', 'sw', 400, 40)
    d.node('DES', '1cv8 DESIGNER', 'sw', 400, 240)
    d.node('AGT', '1cv8 /AgentMode\n127.0.0.1:1543', 'svc', 400, 430, w=180)
    d.node('IBC', 'ibcmd', 'sw', 400, 760)
    d.node('RAC', 'rac', 'sw', 400, 1140)
    d.node('WEB', 'webinst', 'sw', 400, 550)
    d.frame('HOST', 'машина раннера', ['R', 'EDT', 'DES', 'AGT', 'IBC', 'RAC', 'WEB'])
    d.node('EDTSRC', 'исходники\nформат EDT', 'doc', 700, -10)
    d.node('XMLSRC', 'исходники\nXML платформы', 'doc', 700, 100)
    d.node('FDB', 'файловая база\nкаталог с 1Cv8.1CD', 'doc', 1000, 380, w=200, h=72)
    d.node('GATE', 'автономный сервер ibsrv\nпрямой шлюз · SSH 1543', 'svc', 1040, 650, w=230)
    d.note(900, 790, 'кластер и ibsrv — поочерёдно')
    d.node('CLU', 'кластер 1С\nragent · rmngr · rphost', 'sw', 740, 880, w=200)
    d.node('DBMS', 'база\nв СУБД', 'cyl', 1080, 880, w=130, h=60)
    d.node('RAS', 'RAS 1545', 'svc', 850, 1140)
    d.node('WS', 'Apache · IIS', 'svc', 760, 510)
    host_fan(d, [('EDT', 'одна живая сессия', 'r:0.05', 0), ('DES', 'процесс на операцию', 'r:0.22', 0), ('AGT', 'поднимает, потом SSH', 'r:0.4', 0),
                 ('IBC', 'процесс на операцию', 'r:0.8', 0), ('RAC', 'процесс на операцию', 'r:0.97', 0), ('WEB', 'процесс на операцию', 'r:0.6', 0)])

    def route(a, b, label=None, dashed=False, fa='r', fb='l', via=(), at=None, radius=18):
        start, end = d.anchor(a, fa)[:2], d.anchor(b, fb)[:2]
        d.edge(a, b, label, dashed, fa, fb, path=d.rounded_path([start, *via, end], radius), lx=at[0] if at else None, ly=at[1] if at else None)

    route('R', 'GATE', 'SSH · SFTP', True, fa='t', fb='r', via=[(60,-170),(1220,-170),(1220,650)], at=(840,-170))
    route('R', 'RAS', 'поднимает, если cluster.ras не объявлен', fa='b', fb='b', via=[(60,1250),(850,1250)], at=(600,1250))
    d.edge('EDT', 'EDTSRC', fb='l:0.4')
    d.edge('EDT', 'XMLSRC', 'импорт\nэкспорт', fb='l:0.2', lx=570)
    route('DES', 'XMLSRC', fa='r:0.1', fb='l:0.5', via=[(540,222.4),(540,100)])
    # Порт на прямой верхней грани капсулы, а не в углу её bounding box.
    route('AGT', 'XMLSRC', fa='t:0.75', fb='l:0.78', via=[(445,350),(580,350),(580,121.3)])
    d.edge('DES', 'FDB', 'файл', fa='r:0.6', fb='l:0.17', lx=765)
    d.edge('AGT', 'FDB', 'файл', fa='r', fb='l:0.38', lx=790)
    route('DES', 'CLU', 'TCP 1541', True, fa='b:0.2', fb='l:0.25', via=[(362.2,320),(590,320),(590,865.5)], at=(590,725))
    route('AGT', 'CLU', 'TCP 1541', True, fa='b:0.75', fb='l:0.7', via=[(445,510),(540,510),(540,891.6)], at=(590,891.6))
    route('DES', 'GATE', 'TCP: прямой шлюз', True, fa='r:0.35', fb='t:0.78', via=[(1160,233.4),(1160,530),(1104.4,530)], at=(1000,233.4))
    route('GATE', 'FDB', 'файл', fa='t', fb='r:0.42', via=[(1040,510),(1140,510),(1140,372.8)], at=(1080,510), radius=8)
    route('GATE', 'DBMS', 'СУБД', fa='b', fb='t', via=[(1040,810),(1080,810)], at=(1080,828), radius=8)
    route('IBC', 'FDB', '--db-path', fa='r:0.3', fb='l:0.8', via=[(520,751.2),(520,560),(860,560),(860,407)], at=(700,560), radius=8)
    route('IBC', 'DBMS', '--dbms\nкластерная ИБ: кластер остановлен', True, fa='r:0.7', fb='b', via=[(520,768.8),(520,1030),(1080,1030)], at=(880,1030))
    d.edge('RAC', 'RAS', 'TCP 1545: сеансы своей базы', dashed=True, lx=630)
    route('RAS', 'CLU', fa='t', fb='b:0.25', via=[(850,1060),(690,1060)])
    d.edge('CLU', 'DBMS', fa='r', fb='l')
    d.edge('WEB', 'WS', 'публикация', fa='r', fb='l', lx=640)
    route('WS', 'FDB', 'File=…: файл', fa='t', fb='l:0.59', via=[(760,450),(840,450),(840,388.1)], at=(800,450), radius=8)
    route('WS', 'CLU', 'Srvr=…;Ref=…: TCP', True, fa='b', fb='t:0.6', at=(760,785))
    for e in d.edges:
        if not e['path']: e['entry_stub'] = e['exit_stub'] = 32
        if e['a'] == 'R' and e['b'] in ('EDT', 'DES', 'AGT', 'IBC', 'RAC', 'WEB'):
            e['entry_stub'] = d.anchor(e['b'], e['fb'])[0] - 200
            e['label'] = e['label'].replace('одна живая сессия', 'одна живая\nсессия').replace('процесс на операцию', 'процесс\nна операцию').replace('поднимает, потом SSH', 'поднимает,\nпотом SSH')
            e['lx'], e['ly'] = (200 + d.anchor(e['b'], e['fb'])[0]) / 2, d.nodes[e['b']]['y']
    return d

general_map('map')

def layers(id):
    d = diagram(id, 'Слои раннера: адаптеры транспорта, оркестрация, общие слои и платформа')
    d.node('CLI', 'cli\nаргументы, текст и JSON', 'sw', 150, 60)
    d.frame('ad', 'Командная строка', ['CLI'])
    d.node('UC', 'use_cases — оркестрация\nзамок, пайплайн', 'run', 290, 190)
    d.node('CFG', 'config', 'sw', 90, 320)
    d.node('DOM', 'domain', 'sw', 230, 320)
    d.node('CD', 'change_detection', 'sw', 400, 320)
    d.node('PARS', 'parsers', 'sw', 570, 320)
    d.frame('sh', 'Общее', ['CFG', 'DOM', 'CD', 'PARS'])
    d.node('PLAT', 'platform — вызов утилит', 'sw', 780, 190)
    d.node('SUP', 'support — файлы, staging, журналы', 'sw', 780, 320)
    d.edge('CLI', 'UC', fa='b', fb='t:0.3')
    d.edge('UC', 'CFG', fa='b:0.15', fb='t')
    d.edge('UC', 'DOM', fa='b:0.4', fb='t')
    d.edge('UC', 'CD', fa='b:0.65', fb='t')
    d.edge('UC', 'PARS', fa='b:0.9', fb='t')
    d.edge('UC', 'PLAT')
    d.edge('PLAT', 'SUP', fa='b', fb='t')
    return d
layers('layers')

def processes(id):
    d = diagram(id, 'Процессы, которые запускает раннер')
    d.node('R', 'раннер\nCLI', 'run', 450, 270, w=160)
    d.node('D', '1cv8 DESIGNER', 'sw', 450, 50)
    d.node('RS', 'ras cluster\nесли cluster.ras не объявлен', 'svc', 215, 120)
    d.node('I', 'ibcmd', 'sw', 620, 120)
    d.node('RC', 'rac', 'sw', 110, 270)
    d.node('A', 'агент: 1cv8 /AgentMode\nили шлюз ibsrv', 'svc', 790, 270)
    d.node('W', 'webinst', 'sw', 260, 420)
    d.node('E', '1cedtcli', 'sw', 640, 420)
    d.node('C', 'клиент 1С', 'sw', 450, 490)
    d.edge('R', 'D', 'процесс на операцию', fa='t', fb='b', t=0.5)
    d.edge('R', 'RS', 'поднимает на время команды', fa='t:0.12', fb='b:0.85', t=0.5)
    d.edge('R', 'I', 'процесс на операцию', fa='t:0.88', fb='b:0.2', t=0.5)
    d.edge('R', 'RC', 'процесс на операцию', fa='l', fb='r', t=0.5)
    d.edge('R', 'A', 'сессия SSH', fa='r', fb='l', t=0.5)
    d.edge('R', 'W', 'процесс на операцию', fa='b:0.12', fb='t:0.85', t=0.5)
    d.edge('R', 'E', 'одна живая сессия', fa='b:0.88', fb='t:0.15', t=0.5)
    d.edge('R', 'C', 'процесс на запуск', fa='b', fb='t', t=0.5)
    return d
processes('processes')

def mapping(id):
    d = diagram(id, 'Шесть направлений между исходниками, базой и пакетом')
    d.node('CFSRC', 'src/cf\nконфигурация', 'doc', 120, 80)
    d.node('EXTSRC', 'src/ext\nрасширение', 'doc', 120, 360)
    d.node('PKG', 'пакет\n.cf · .cfe', 'doc', 120, 220)
    d.frame('FS', 'каталог проекта', ['CFSRC', 'EXTSRC', 'PKG'])
    d.node('MAIN', 'основная\nконфигурация', 'sw', 520, 80)
    d.node('DB', 'конфигурация\nбазы данных', 'run', 800, 80)
    d.node('VEN', 'конфигурации\nпоставщиков', 'off', 800, 220)
    d.node('EMAIN', 'конфигурация\nрасширения', 'sw', 520, 360)
    d.node('EDB', 'база данных\nрасширения', 'run', 800, 360)
    d.frame('IB', 'информационная база', ['MAIN', 'DB', 'VEN', 'EMAIN', 'EDB'])
    d.edge('CFSRC', 'MAIN', 'push main', fa='r:0.3', fb='l:0.3', t=0.5)
    d.edge('MAIN', 'CFSRC', 'pull main', fa='l:0.7', fb='r:0.7', t=0.5)
    d.edge('MAIN', 'DB', 'apply', fa='r:0.3', fb='l:0.3')
    d.edge('DB', 'MAIN', 'reset', fa='l:0.7', fb='r:0.7')
    d.edge('EXTSRC', 'EMAIN', 'push my-ext', fa='r:0.3', fb='l:0.3', t=0.5)
    d.edge('EMAIN', 'EXTSRC', 'pull my-ext', fa='l:0.7', fb='r:0.7', t=0.5)
    d.edge('EMAIN', 'EDB', 'apply my-ext', fa='r:0.3', fb='l:0.3')
    d.edge('EDB', 'EMAIN', 'reset my-ext', fa='l:0.7', fb='r:0.7')
    d.edge('CFSRC', 'PKG', 'make', fa='b', fb='t')
    d.edge('PKG', 'MAIN', 'upload', fa='r:0.3', fb='b:0.3', t=0.45)
    d.edge('MAIN', 'PKG', 'download', fa='b:0.7', fb='r:0.75', t=0.55)
    d.edge('MAIN', 'VEN', 'diff --against vendor:имя', dashed=True, fa='b:0.9', fb='l', t=0.5)
    return d
mapping('mapping')

def states_pair(id):
    d = diagram(id, 'Четыре состояния пары выгрузка и основная конфигурация: совпадают, исходники впереди, база впереди, разошлись')
    d.node('S', 'совпадают', 'sw', 95, 150, w=120)
    d.node('L', 'исходники впереди', 'sw', 380, 58, w=180)
    d.node('B', 'база впереди', 'sw', 380, 242, w=150)
    d.node('D', 'разошлись', 'sw', 660, 150, w=120)
    d.edge('S', 'L', 'правка в файлах', path='M95,128 C95,58 190,58 288,58', t=0.5, dy=-18)
    d.edge('L', 'S', 'push', path='M300,80 C250,80 230,142 157,142', t=0.5)
    d.edge('S', 'B', 'правка в Конфигураторе', path='M95,172 C95,242 190,242 303,242', t=0.5, dy=20)
    d.edge('B', 'S', 'pull', path='M315,220 C260,220 240,158 157,158', t=0.5)
    d.edge('L', 'D', 'правка в Конфигураторе', path='M472,58 C560,58 660,60 660,126', t=0.4, dy=-18)
    d.edge('B', 'D', 'правка в файлах', path='M457,242 C560,242 660,240 660,174', t=0.4, dy=20)
    d.edge('D', 'L', 'pull', path='M598,150 C540,150 530,74 474,74', t=0.5)
    d.edge('D', 'D', 'push отклоняется', path='M720,140 C790,120 790,180 722,160', t=0.5, dx=100)
    return d
states_pair('states-pair')

def states_inside(id):
    d = diagram(id, 'Состояния основной конфигурации и конфигурации базы данных внутри базы')
    d.node('S2', 'совпадают', 'sw', 90, 120, w=120)
    d.node('P', 'есть непринятое', 'sw', 430, 120, w=160)
    d.node('Y', 'обновлено динамически', 'sw', 430, 270, w=210)
    d.edge('S2', 'P', 'push --no-apply', path='M150,102 C230,26 310,26 350,102', t=0.5)
    d.edge('S2', 'P', 'правка в Конфигураторе', path='M150,112 C230,72 320,72 350,112', t=0.5)
    d.edge('S2', 'P', 'upload', fa='r:0.62', fb='l:0.62', t=0.5)
    d.edge('P', 'S2', 'apply', path='M350,136 C300,180 200,180 150,136', t=0.5)
    d.edge('P', 'S2', 'reset', path='M350,141 C300,236 200,236 150,141', t=0.5)
    d.edge('P', 'Y', 'apply,\nкогда сеансы не закрыть', fa='b', fb='t', t=0.5)
    d.edge('Y', 'S2', 'перезапуск сеансов', path='M325,270 C200,270 90,250 90,142', t=0.3)
    return d
states_inside('states-inside')

# ---------------- расстановки ----------------
FAN = 'процесс на операцию'

def legend(id):
    d = diagram(id, 'Как читать схемы: процесс, точка входа, файл, база данных, раннер, вне раннера')
    d.node('L5', 'v8-runner', 'run', 110, 50, w=150)
    d.node('L1', 'процесс\nутилита платформы', 'sw', 340, 50, w=170)
    d.node('L2', 'точка входа\nSSH-шлюз, HTTP', 'svc', 590, 50, w=190)
    d.node('L3', 'файл или каталог', 'doc', 110, 140, w=150)
    d.node('L4', 'база данных СУБД', 'cyl', 340, 140, w=160, h=54)
    d.node('L6', 'вне раннера', 'off', 590, 140, w=150)
    return d
legend('legend')

def d_file(id):
    d = diagram(id, 'Файловая база: все утилиты и база на машине раннера')
    d.node('R', 'v8-runner', 'run', 80, 200, w=130)
    d.node('DES', '1cv8 DESIGNER', 'sw', 380, 60)
    d.node('IBC', 'ibcmd', 'sw', 380, 150)
    d.node('AGT', '1cv8 /AgentMode\n127.0.0.1:1543', 'svc', 380, 250)
    d.node('CLI', '1cv8c · клиент', 'sw', 380, 350)
    d.node('IB', 'каталог базы\n1Cv8.1CD', 'doc', 720, 200, w=170)
    d.node('WP', 'workPath\nжурналы, хеши, .cf, .dt', 'doc', 720, 60, w=210)
    d.frame('HOST', 'машина раннера', ['R', 'DES', 'IBC', 'AGT', 'CLI', 'IB', 'WP'])
    host_fan(d, [('DES', FAN, 'r:0.15', 0.7), ('IBC', FAN, 'r:0.4', 0.7), ('AGT', 'поднимает, потом SSH', 'r:0.65', 0.5), ('CLI', 'процесс на запуск', 'r:0.9', 0.7)])
    d.edge('DES', 'IB', 'файл, монопольно', fa='r:0.75', fb='l:0.2', t=0.5)
    d.edge('IBC', 'IB', '--db-path', fb='l:0.45')
    d.edge('AGT', 'IB', 'файл', fa='r:0.3', fb='l:0.75', t=0.5)
    d.edge('CLI', 'IB', 'файл', fb='b:0.3', t=0.5)
    d.edge('DES', 'WP', fa='r:0.35', fb='l:0.5')
    return d
d_file('d-file')

def d_file_web(id):
    d = diagram(id, 'Файловая база и веб-сервер на машине раннера')
    d.node('R', 'v8-runner', 'run', 80, 220, w=130)
    d.node('WEBI', 'webinst', 'sw', 380, 80)
    d.node('DES', '1cv8 DESIGNER', 'sw', 380, 220)
    d.node('CLI', '1cv8c · клиент', 'sw', 380, 340)
    d.node('WS', 'Apache · IIS\nмодуль wsap · wsisapi', 'svc', 900, 80, w=230)
    d.node('PUB', 'каталог публикации\ndefault.vrd', 'doc', 900, 220, w=190)
    d.node('IB', 'каталог базы\n1Cv8.1CD', 'doc', 900, 360, w=170)
    d.frame('HOST', 'машина раннера', ['R', 'WEBI', 'DES', 'CLI', 'WS', 'PUB', 'IB'],
            extra=((1140, 460, 1140, 460),))
    d.node('BR', 'браузер · тонкий клиент', 'svc', 900, -110)
    host_fan(d, [('WEBI', FAN, 'r:0.2', 0.7), ('DES', FAN, 'r:0.5', 0.6), ('CLI', 'процесс на запуск', 'r:0.8', 0.7)])
    for edge in d.edges:
        edge['exit_stub'] = edge['entry_stub'] = 30
    # Labels occupy separate horizontal corridors, with clear arrowheads.
    sx, sy, _ = d.anchor('WEBI', 'b')
    tx, ty, _ = d.anchor('PUB', 'l:0.3')
    d.edge('WEBI', 'PUB', '-dir, -wsdir, -connstr File=…', fa='b', fb='l:0.3',
           path=d.rounded_path([(sx, sy), (sx, 150), (730, 150), (730, ty), (tx, ty)]), lx=560, ly=150)
    for source, port, turn, label in [('DES', 'l:0.35', 730, 'файл, монопольно'),
                                      ('CLI', 'l:0.7', 650, 'File=…, файл')]:
        sx, sy, _ = d.anchor(source, 'r')
        tx, ty, _ = d.anchor('IB', port)
        d.edge(source, 'IB', label, fb=port,
               path=d.rounded_path([(sx, sy), (turn, sy), (turn, ty), (tx, ty)]), lx=570, ly=sy)
    d.edge('WEBI', 'WS', '-confpath: правка конфига', lx=600, entry_stub=30, exit_stub=30)
    sx, sy, _ = d.anchor('WS', 'b:0.8')
    tx, ty, _ = d.anchor('IB', 'r')
    d.edge('WS', 'IB', 'файл', fa='b:0.8', fb='r',
           path=d.rounded_path([(sx, sy), (sx, 170), (1060, 170), (1060, ty), (tx, ty)]), lx=1060, ly=270)
    d.edge('WS', 'PUB', fa='b', fb='t', entry_stub=30, exit_stub=30)
    d.edge('BR', 'WS', 'HTTP · HTTPS, ws=…', dashed=True, fa='b', fb='t', lx=900, ly=-25,
           entry_stub=30, exit_stub=30)
    sx, sy, _ = d.anchor('CLI', 'b')
    tx, ty, _ = d.anchor('WS', 'r')
    d.edge('CLI', 'WS', 'HTTP · HTTPS, ws=…', dashed=True, fa='b', fb='r',
           path=d.rounded_path([(sx, sy), (sx, 460), (1120, 460), (1120, ty), (tx, ty)]), lx=650, ly=460)
    return d

d_file_web('d-file-web')

def d_cluster_stopped(id, title, hosting):
    """Прямой SQL-канал: процессы обслуживающего кластера остановлены."""
    d = diagram(id, title + ' — ibcmd при остановленном кластере')
    d.node('R', 'v8-runner', 'run', 100, 160, w=130)
    d.node('IBC', 'ibcmd', 'sw', 380, 160)
    d.node('WP', 'workPath\nжурналы и артефакты', 'doc', 380, 300, w=200)
    d.node('CL', 'ragent · rmngr · rphost\nостановлены', 'off', 900, 300, w=260)
    address = 'db:5432' if hosting == 'remote' else 'localhost:5432'
    d.node('DBMS', 'PostgreSQL\n' + address, 'cyl', 900, 160, w=150, h=60)
    d.frame('HOST', 'машина раннера', ['R', 'IBC', 'WP'])
    if hosting == 'local':
        d.frame('BOX', 'одна машина', ['R', 'IBC', 'WP', 'CL', 'DBMS'], kind='box', pad=44, top=52)
    elif hosting == 'docker':
        d.frame('C1', 'контейнер сервера 1С остановлен', ['CL'], kind='neutral')
        d.frame('C2', 'контейнер СУБД, порт опубликован на хосте', ['DBMS'], kind='neutral')
    else:
        d.frame('SRV', 'сервер приложений srv', ['CL'], kind='neutral')
        d.frame('SQL', 'сервер СУБД db', ['DBMS'], kind='srv')
    d.edge('R', 'IBC', FAN, lx=248)
    d.edge('IBC', 'WP', 'файлы', fa='b', fb='t')
    d.edge('IBC', 'DBMS', 'dbms: прямой доступ', dashed=hosting != 'local', lx=630)
    return d

def d_cluster_local(id, dbms=False):
    if dbms:
        return d_cluster_stopped(id, 'Кластер на машине раннера: раннер, утилиты и сервер 1С на одной машине', 'local')
    d = diagram(id, 'Кластер на машине раннера: раннер, утилиты и сервер 1С на одной машине')
    d.node('R', 'v8-runner', 'run', 80, 220, w=130)
    d.node('DES', '1cv8 DESIGNER', 'sw', 380, 80)
    d.node('AGT', '1cv8 /AgentMode\n127.0.0.1:1543', 'svc', 380, 175)
    d.node('CLI', '1cv8c · клиент', 'sw', 380, 275)
    d.node('WP', 'workPath\n.cf, .dt, журналы', 'doc', 620, 50, w=170)
    d.frame('HOST', 'раннер и утилиты', ['R', 'DES', 'AGT', 'CLI', 'WP'])
    d.node('RAG', 'ragent 1540\nrmngr 1541', 'sw', 900, 170)
    d.node('RPH', 'rphost 1560–1591', 'sw', 900, 275)
    d.frame('SRV', 'сервер 1С', ['RAG', 'RPH'], kind='srv')
    d.frame('BOX', 'одна машина', ['R', 'DES', 'AGT', 'CLI', 'WP', 'RAG', 'RPH', 'DBMS'], kind='box', pad=44, top=52)
    d.node('DBMS', 'PostgreSQL\nlocalhost:5432', 'cyl', 1140, 275, w=150, h=60)
    host_fan(d, [('DES', FAN, 'r:0.15', 0.7), ('AGT', 'поднимает, потом SSH', 'r:0.4', 0.5), ('CLI', 'процесс на запуск', 'r:0.65', 0.7)])
    d.edge('DES', 'WP', fa='r:0.3', fb='l')
    d.edge('DES', 'RAG', 'Srvr=…;Ref=…, TCP', path='M445,90 C560,140 770,150 844,160', lx=600)
    d.edge('AGT', 'RAG', 'TCP', fa='r:0.4', fb='l:0.5', t=0.6)
    d.edge('CLI', 'RAG', 'TCP', fb='l:0.85', t=0.6)
    d.edge('RAG', 'RPH', fa='b', fb='t')
    d.edge('RPH', 'DBMS')
    d.title += ' — через работающий кластер'
    return d

d_cluster_local('d-cluster-local')
d_cluster_local('d-cluster-local-dbms', dbms=True)

def d_cluster_remote(id, dbms=False):
    if dbms:
        return d_cluster_stopped(id, 'Кластер на другой машине с несколькими rphost', 'remote')
    d = diagram(id, 'Кластер на другой машине с несколькими rphost')
    d.node('R', 'v8-runner', 'run', 100, 200, w=130)
    d.node('DES', '1cv8 DESIGNER', 'sw', 380, 100)
    d.node('AGT', '1cv8 /AgentMode\n127.0.0.1:1543', 'svc', 380, 330)
    d.node('CLI', '1cv8c · клиент', 'sw', 380, 220)
    d.node('WP', 'workPath\n.cf, .dt, отчёты тестов', 'doc', 620, 30, w=210)
    d.frame('HOST', 'машина раннера', ['R', 'DES', 'CLI', 'AGT', 'WP'])
    d.node('RAG', 'ragent 1540\nrmngr 1541', 'sw', 960, 220, w=220, h=80)
    d.node('RP1', 'rphost', 'sw', 840, 340)
    d.node('RP2', 'rphost', 'sw', 960, 340)
    d.node('RP3', 'rphost', 'sw', 1080, 340)
    d.node('LOG', 'журнал регистрации,\nтехнологический журнал', 'docoff', 960, 470, w=220)
    d.frame('SRV', 'сервер приложений 1С', ['RAG', 'RP1', 'RP2', 'RP3', 'LOG'], kind='srv')
    d.node('DBMS', 'PostgreSQL\ndb:5432', 'cyl', 1320, 340, w=110, h=60)
    host_fan(d, [('DES', FAN, 'r:0.25', 0.7), ('CLI', 'процесс на запуск', 'r:0.5', 0.7)])
    d.edge('R', 'AGT', 'поднимает, потом SSH', fa='b', lx=215)
    d.edge('DES', 'WP', fa='r:0.3', fb='l')
    # Отдельные горизонтальные коридоры: подпись лежит на своей линии,
    # а повороты и порты вычисляются из текущих границ блоков.
    for source, port, turn_x, label in [
        ('DES', 'l:0.25', 740, 'TCP 1541, дальше 1560–1591'),
        ('CLI', 'l', None, 'TCP'),
        ('AGT', 'l:0.75', 740, 'TCP'),
    ]:
        start = d.anchor(source, 'r')[:2]
        end = d.anchor('RAG', port)[:2]
        via = [(turn_x, start[1]), (turn_x, end[1])] if turn_x else []
        d.edge(source, 'RAG', label, dashed=True, fb=port,
               path=d.rounded_path([start, *via, end]), lx=620, ly=start[1])
    d.edge('RAG', 'RP1', fa='b:0.2', fb='t')
    d.edge('RAG', 'RP2', fa='b:0.5', fb='t')
    d.edge('RAG', 'RP3', fa='b:0.8', fb='t')
    d.edge('RP1', 'DBMS', fa='b', fb='l:0.8', path='M840,362 C840,420 1140,420 1265,358')
    d.edge('RP2', 'DBMS', fa='b', fb='l:0.7', path='M960,362 C960,405 1150,410 1265,352')
    d.edge('RP3', 'DBMS', fb='l:0.3')
    d.edge('RP1', 'LOG', fa='b:0.4', fb='t:0.2')
    d.title += ' — через работающий кластер'
    return d

d_cluster_remote('d-cluster-remote')
d_cluster_remote('d-cluster-remote-dbms', dbms=True)

def d_cluster_ha(id):
    d = diagram(id, 'Отказоустойчивый кластер на двух центральных серверах: srv1 и srv2')
    d.node('R', 'v8-runner', 'run', 100, 300, w=130)
    d.node('DES', '1cv8 DESIGNER', 'sw', 400, 140)
    d.node('AGT', '1cv8 /AgentMode\n127.0.0.1:1543', 'svc', 400, 300)
    d.node('CLI', '1cv8c · клиент', 'sw', 400, 460)
    d.node('WP', 'workPath\n.cf, .dt, отчёты тестов', 'doc', 650, 80, w=210)
    d.frame('HOST', 'машина раннера', ['R', 'DES', 'AGT', 'CLI', 'WP'])
    for suffix, y in [('1', 190), ('2', 460)]:
        d.node('ENTRY' + suffix, 'Вход в кластер\nsrv' + suffix + ':1541', 'svc', 1050, y, w=190)
        d.node('RP' + suffix, 'rphost', 'sw', 1310, y)
        d.frame('SRV' + suffix, 'центральный сервер srv' + suffix,
                ['ENTRY' + suffix, 'RP' + suffix], kind='srv')
        d.edge('ENTRY' + suffix, 'RP' + suffix)
    d.frame('CLUSTER', 'один отказоустойчивый кластер',
            ['ENTRY1', 'RP1', 'ENTRY2', 'RP2'], kind='box', pad=44, top=70)
    d.node('DBMS', 'PostgreSQL\ndb:5432', 'cyl', 1580, 325, w=130, h=60)
    host_fan(d, [('DES', FAN, 'r:0.2', 0.7),
                 ('AGT', 'поднимает, потом SSH', 'r:0.5', 0.6),
                 ('CLI', 'процесс на запуск', 'r:0.8', 0.7)])
    d.edge('DES', 'WP', fa='r:0.15', fb='l')
    for utility in ['DES', 'AGT', 'CLI']:
        # Side ports of a capsule lie on its outline only at the midpoint.
        first_port = 'r' if utility == 'AGT' else 'r:0.35'
        second_port = 'r' if utility == 'AGT' else 'r:0.75'
        d.edge(utility, 'ENTRY1', dashed=True, fa=first_port, fb='l')
        d.edge(utility, 'ENTRY2', dashed=True, fa=second_port, fb='l')
    d.edge('RP1', 'DBMS', fb='l:0.3')
    d.edge('RP2', 'DBMS', fb='l:0.7')
    return d

d_cluster_ha('d-cluster-ha')

def d_ras(id):
    d = diagram(id, 'Кластер, RAS поднят удалённо: раннер ходит к нему утилитой rac за сеансами своей базы')
    d.node('R', 'v8-runner', 'run', 100, 170, w=130)
    d.node('DES', '1cv8 DESIGNER', 'sw', 380, 80)
    d.node('RAC', 'rac', 'sw', 380, 260)
    d.frame('HOST', 'машина раннера', ['R', 'DES', 'RAC'])
    d.node('RAG', 'ragent 1540 · rmngr 1541', 'sw', 800, 80, w=220)
    d.node('RPH', 'rphost 1560–1591', 'sw', 1053, 80, w=150)
    d.frame('SRV', 'сервер 1С', ['RAG', 'RPH'], kind='srv')
    d.node('RAS', 'RAS 1545', 'svc', 800, 260, w=220)
    d.node('CON', 'консоль кластера', 'off', 1053, 260, w=150)
    d.frame('ADMIN', 'машина с RAS', ['RAS', 'CON'], kind='neutral')
    host_fan(d, [('DES', FAN, 'r:0.25', 0.6), ('RAC', FAN, 'r:0.75', 0.6)])
    d.edge('DES', 'RAG', 'TCP 1541, Srvr=…;Ref=…', dashed=True, fb='l:0.5', t=0.5)
    d.edge('RAC', 'RAS', 'TCP 1545, cluster.ras', dashed=True, t=0.5)
    d.edge('RAS', 'RAG', 'TCP 1540', dashed=True, fa='t', fb='b', t=0.6)
    d.edge('CON', 'RAS', fa='l', fb='r')
    d.edge('RAG', 'RPH')
    return d
d_ras('d-ras')

def d_ras_managed(id):
    d = diagram(id, 'Кластер, RAS поднят раннером на своей машине на время команды')
    d.node('R', 'v8-runner', 'run', 100, 200, w=130)
    d.node('DES', '1cv8 DESIGNER', 'sw', 380, 80)
    d.node('RAC', 'rac', 'sw', 380, 200)
    d.node('RAS', 'ras cluster\n127.0.0.1:1545, поднят раннером', 'svc', 380, 330, w=270)
    d.frame('HOST', 'машина раннера', ['R', 'DES', 'RAC', 'RAS'])
    d.node('RAG', 'ragent 1540 · rmngr 1541', 'sw', 880, 80, w=220)
    d.node('RPH', 'rphost 1560–1591', 'sw', 1133, 80, w=150)
    d.frame('SRV', 'сервер 1С', ['RAG', 'RPH'], kind='srv')
    host_fan(d, [('DES', FAN, 'r:0.2', 0.7), ('RAC', FAN, 'r:0.5', 0.6), ('RAS', 'поднимает на время команды', 'r:0.85', 0.5)])
    d.edge('RAC', 'RAS', 'TCP 1545', fa='b', fb='t:0.5', t=0.5, dx=40)
    d.edge('DES', 'RAG', 'TCP 1541, Srvr=…;Ref=…', dashed=True, fb='l:0.5', t=0.6)
    d.edge('RAS', 'RAG', 'TCP 1540, хост из Srvr=', dashed=True, path='M515,330 C700,330 836,250 836,102', t=0.25)
    d.edge('RAG', 'RPH')
    return d
d_ras_managed('d-ras-managed')

def d_cluster_web(id):
    d = diagram(id, 'Кластер и веб-публикация на машине раннера')
    d.node('R', 'v8-runner', 'run', 80, 360, w=130)
    d.node('WEBI', 'webinst', 'sw', 380, 80)
    d.node('DES', '1cv8 DESIGNER', 'sw', 380, 240)
    d.node('AGT', '1cv8 /AgentMode\n127.0.0.1:1543', 'svc', 380, 400)
    d.node('CLI', '1cv8c · клиент', 'sw', 380, 680)
    d.node('WP', 'workPath\n.cf, .dt, журналы', 'doc', 220, 160, w=170)
    d.node('PUB', 'каталог публикации\ndefault.vrd', 'doc', 720, 80, w=190)
    d.node('WS', 'Apache 2.4\nмодуль wsap', 'svc', 700, 540, w=190)
    d.frame('HOST', 'машина раннера', ['R', 'WEBI', 'DES', 'AGT', 'CLI', 'WP', 'WS', 'PUB'],
            extra=((880, 750, 880, 750),))
    d.node('RAG', 'ragent · rmngr 1541', 'sw', 1080, 400, w=190, h=160)
    d.node('RPH', 'rphost', 'sw', 1080, 660)
    d.frame('SRV', 'сервер приложений srv', ['RAG', 'RPH'], kind='srv')
    d.node('DBMS', 'PostgreSQL\ndb:5432', 'cyl', 1080, 800, w=130, h=60)
    d.node('BR', 'браузер · тонкий клиент', 'svc', 700, 800)
    # Launch labels sit on individual approaches instead of the shared fan.
    for target, face, via, label, lx, ly in [
        ('WEBI', 't', [(80, 80)], FAN, 240, 80),
        ('DES', 'r:0.2', [(200, 346.8), (200, 240)], FAN, 231, 240),
        ('AGT', 'r', [(200, 360), (200, 400)], 'SSH', 240, 400),
        ('CLI', 'b', [(80, 680)], 'процесс на запуск', 220, 680),
    ]:
        start = d.anchor('R', face)[:2]
        end = d.anchor(target, 'l')[:2]
        d.edge('R', target, label, fa=face, path=d.rounded_path([start, *via, end]), lx=lx, ly=ly)
    d.edge('WEBI', 'PUB', '-dir, -wsdir, -connstr', exit_stub=30, entry_stub=30)
    start = d.anchor('WEBI', 'b')[:2]
    end = d.anchor('WS', 'l')[:2]
    d.edge('WEBI', 'WS', '-confpath', fa='b',
           path=d.rounded_path([start, (380, 150), (500, 150), (500, 540), end]), lx=500, ly=300)
    start = d.anchor('DES', 't')[:2]
    end = d.anchor('WP', 'r')[:2]
    d.edge('DES', 'WP', fa='t', fb='r', path=d.rounded_path([start, (380, 160), end]))
    # Four distinct ports and departure labels make network paths traceable.
    for source, port, turn, label, lx in [
        ('DES', 'l:0.125', 890, 'TCP 1541', 800),
        ('AGT', 'l:0.375', 890, 'TCP', 800),
        ('WS', 'l:0.625', 870, 'TCP', 830),
        ('CLI', 'l:0.875', 930, 'TCP', 570),
    ]:
        start = d.anchor(source, 'r')[:2]
        end = d.anchor('RAG', port)[:2]
        d.edge(source, 'RAG', label, dashed=True, fb=port,
               path=d.rounded_path([start, (turn, start[1]), (turn, end[1]), end]), lx=lx, ly=start[1])
    # Enter the publication from below on the same vertical as Apache.
    pub_face = f"b:{0.5 + (d.nodes['WS']['x'] - d.nodes['PUB']['x']) / d.nodes['PUB']['w']}"
    start = d.anchor('WS', 't')[:2]
    end = d.anchor('PUB', pub_face)[:2]
    d.edge('WS', 'PUB', fa='t', fb=pub_face, path=d.rounded_path([start, end]))
    # The local thin client can use HTTP instead of its direct cluster channel.
    start = d.anchor('CLI', 't')[:2]
    end = d.anchor('WS', 'b:0.2')[:2]
    d.edge('CLI', 'WS', 'HTTP', dashed=True, fa='t', fb='b:0.2',
           path=d.rounded_path([start, (380, 614), (643, 614), end], radius=10), lx=500, ly=614)
    start = d.anchor('BR', 't')[:2]
    end = d.anchor('WS', 'b:0.8')[:2]
    d.edge('BR', 'WS', 'HTTP', dashed=True, fa='t', fb='b:0.8',
           path=d.rounded_path([start, (700, 710), (757, 710), end]), lx=757, ly=640)
    for source, target in [('RAG', 'RPH'), ('RPH', 'DBMS')]:
        start = d.anchor(source, 'b')[:2]
        end = d.anchor(target, 't')[:2]
        d.edge(source, target, fa='b', fb='t', path=d.rounded_path([start, end]))
    d.title += ' — через работающий кластер'
    return d

d_cluster_web('d-cluster-web')

def d_standalone_local(id):
    d = diagram(id, 'Автономный сервер на машине раннера: прямой шлюз для Конфигуратора и SSH-шлюз для агентского набора')
    d.node('R', 'v8-runner', 'run', 100, 160, w=130)
    d.node('DES', '1cv8 DESIGNER', 'sw', 430, 160)
    d.node('WP', 'workPath\nвсегда локален', 'doc', 100, 300, w=150)
    d.frame('HOST', 'раннер', ['R', 'WP', 'DES'], extra=((100, 60, 100, 60),))
    d.node('GATE', 'SSH-шлюз 1543\nibsrv --enable-ssh-gate', 'svc', 760, 80, w=250)
    d.node('DGATE', 'прямой шлюз\nSrvr=localhost;Ref=demo', 'svc', 760, 190, w=250)
    d.node('IBSRV', 'ibsrv\nвне раннера', 'sw', 1060, 190)
    d.node('UD', 'каталог пользователя шлюза\nusers-data и имя пользователя', 'doc', 1060, 330, w=260)
    d.node('HTTPS', 'HTTP 8314', 'svc', 1060, 80)
    d.node('DB', 'база данных', 'cyl', 1320, 190, w=120, h=54)
    d.frame('SRV', 'автономный сервер', ['GATE', 'DGATE', 'IBSRV', 'UD', 'HTTPS', 'DB'], kind='srv')
    d.frame('BOX', 'одна машина', ['R', 'WP', 'DES', 'GATE', 'DGATE', 'IBSRV', 'UD', 'HTTPS', 'DB'], kind='box', pad=44, top=52)
    d.node('BR', 'браузер · тонкий клиент', 'svc', 1060, -90)
    d.edge('R', 'GATE', 'SSH: команда строкой, ответ JSON', path='M100,138 C100,85 180,80 635,80', t=0.7)
    d.edge('R', 'DES', FAN, t=0.5)
    d.edge('R', 'UD', 'exchange: dir\nссылки и копии', path='M165,176 C300,176 500,330 930,330', t=0.45)
    d.edge('R', 'WP', 'файл', fa='b', fb='t')
    d.edge('DES', 'DGATE', 'TCP', fb='l:0.5', t=0.5)
    d.edge('DGATE', 'IBSRV')
    d.edge('GATE', 'IBSRV', path='M885,90 C950,90 990,170 1000,178')
    d.edge('IBSRV', 'UD', fa='b', fb='t')
    d.edge('IBSRV', 'DB')
    d.edge('IBSRV', 'HTTPS', fa='t', fb='b')
    d.edge('BR', 'HTTPS', fa='b', fb='t')
    return d

d_standalone_local('d-standalone-local')

def d_standalone_remote(id):
    d = diagram(id, 'Автономный сервер на другой машине: Конфигуратор по прямому шлюзу, SSH и SFTP для агентского пути')
    d.node('R', 'v8-runner', 'run', 100, 220, w=130)
    d.node('WP', 'workPath\nостаётся здесь', 'doc', 100, 360, w=150)
    d.node('DES', '1cv8 DESIGNER', 'sw', 400, 220)
    d.node('SRC', 'исходники и артефакты', 'doc', 400, 340, w=200)
    d.frame('HOST', 'машина раннера', ['R', 'WP', 'SRC', 'DES'], extra=((100, 60, 100, 60),))
    d.node('GATE', 'SSH-шлюз 1543', 'svc', 820, 80)
    d.node('DGATE', 'прямой шлюз', 'svc', 820, 190)
    d.node('IBSRV', 'ibsrv', 'sw', 1080, 190)
    d.node('UD', 'каталог пользователя шлюза\nпути разрешаются здесь', 'doc', 1080, 340, w=250)
    d.node('HTTPS', 'HTTP 8314', 'svc', 1080, 80)
    d.frame('SRV', 'другая машина', ['GATE', 'DGATE', 'IBSRV', 'UD', 'HTTPS'], kind='srv')
    d.node('DB', 'база данных', 'cyl', 1350, 190, w=120, h=54)
    d.node('BR', 'браузер · тонкий клиент', 'svc', 1350, 80)
    d.edge('R', 'WP', 'файл', fa='b', fb='t')
    d.edge('R', 'SRC', 'файл', fa='r:0.85', fb='l:0.5', t=0.5)
    d.edge('R', 'GATE', 'SSH: команда строкой, ответ JSON', dashed=True, path='M100,198 C100,85 180,80 746,80', t=0.7)
    d.edge('R', 'DES', FAN, fa='r:0.5', t=0.45)
    d.edge('DES', 'DGATE', 'TCP', dashed=True, t=0.5)
    d.edge('DGATE', 'IBSRV')
    d.edge('SRC', 'UD', 'SFTP того же соединения', dashed=True, lx=623)
    d.edge('GATE', 'IBSRV', path='M890,90 C960,90 1010,170 1020,178')
    d.edge('IBSRV', 'UD', fa='b', fb='t')
    d.edge('IBSRV', 'DB')
    d.edge('IBSRV', 'HTTPS', fa='t', fb='b')
    d.edge('BR', 'HTTPS', dashed=True, fa='l', fb='r')
    return d

d_standalone_remote('d-standalone-remote')

def d_standalone_managed(id):
    d = diagram(id, 'Автономный сервер под управлением раннера: ibsrv поднят на машине раннера командой standalone start')
    d.node('R', 'v8-runner', 'run', 100, 160, w=130)
    d.node('DES', '1cv8 DESIGNER', 'sw', 430, 160)
    d.node('WP', 'workPath\nзапись о сервере, ключ хоста', 'doc', 100, 300, w=230)
    d.frame('HOST', 'раннер', ['R', 'WP', 'DES'], extra=((100, 60, 100, 60),))
    d.node('GATE', 'SSH-шлюз\nсвободный порт, ключ раннера', 'svc', 760, 80, w=250)
    d.node('DGATE', 'прямой шлюз\nсвободный порт, проба готовности', 'svc', 760, 190, w=270)
    d.node('IBSRV', 'ibsrv\nподнят раннером', 'sw', 1060, 190)
    d.node('HTTPS', 'HTTP\nсвободный порт', 'svc', 1060, 80)
    d.node('DB', 'файловая база\nmanaged.db-path', 'cyl', 1300, 190, w=170, h=60)
    d.frame('SRV', 'автономный сервер', ['GATE', 'DGATE', 'IBSRV', 'HTTPS', 'DB'], kind='srv')
    d.frame('BOX', 'одна машина', ['R', 'WP', 'DES', 'GATE', 'DGATE', 'IBSRV', 'HTTPS', 'DB'], kind='box', pad=44, top=52)
    d.edge('R', 'IBSRV', 'standalone start · stop', path='M165,178 C420,262 880,262 1060,212', lx=330)
    d.edge('R', 'GATE', 'SSH: команда строкой, ответ JSON', path='M100,138 C100,85 180,80 635,80', t=0.7)
    d.edge('R', 'DES', FAN, t=0.5)
    d.edge('R', 'WP', 'файл', fa='b', fb='t')
    d.edge('DES', 'DGATE', 'TCP', fb='l:0.5', t=0.5)
    d.edge('DGATE', 'IBSRV')
    d.edge('GATE', 'IBSRV', path='M885,90 C950,90 990,170 1000,178')
    d.edge('IBSRV', 'DB')
    d.edge('IBSRV', 'HTTPS', fa='t', fb='b')
    return d

d_standalone_managed('d-standalone-managed')

def d_cluster_managed(id):
    d = diagram(id, 'Кластер под управлением раннера: сервер 1С запущен раннером как приложение на его машине')
    d.node('R', 'v8-runner', 'run', 100, 160, w=130)
    d.node('DES', '1cv8 DESIGNER', 'sw', 400, 80)
    d.node('CLI', '1cv8c · клиент', 'sw', 400, 240)
    d.node('WP', 'workPath\nкаталог данных кластера,\nзапись о процессе', 'doc', 100, 340, w=230, h=86)
    d.frame('HOST', 'раннер и утилиты', ['R', 'DES', 'CLI', 'WP'])
    d.node('RAG', 'ragent · rmngr\nподняты раннером, свободные порты', 'sw', 870, 160, w=290, h=70)
    d.node('RPH', 'rphost\nсвободный диапазон', 'sw', 870, 320, w=200)
    d.frame('CLU', 'сервер 1С', ['RAG', 'RPH'], kind='srv')
    d.node('DBMS', 'ваша СУБД\nPostgreSQL localhost:5432', 'cyl', 1220, 320, w=200, h=60)
    d.frame('BOX', 'одна машина: Windows или Linux', ['R', 'DES', 'CLI', 'WP', 'RAG', 'RPH', 'DBMS'], kind='box', pad=44, top=52)
    host_fan(d, [('DES', FAN, 'r:0.25', 0.7), ('CLI', 'процесс на запуск', 'r:0.75', 0.7)])
    d.edge('R', 'WP', 'файл', fa='b', fb='t')
    d.edge('R', 'RAG', 'cluster start · stop', path='M165,182 C260,430 640,430 760,195', lx=300)
    d.edge('DES', 'RAG', 'TCP, Srvr=localhost:<порт>', fa='r', fb='l:0.3', t=0.5)
    d.edge('CLI', 'RAG', 'TCP', fa='r', fb='l:0.7', t=0.5)
    d.edge('RAG', 'RPH', fa='b', fb='t')
    d.edge('RPH', 'DBMS')
    return d

d_cluster_managed('d-cluster-managed')

def d_console(id):
    d = diagram(id, 'Кластер поднят из консоли, а не службой')
    d.node('R', 'v8-runner', 'run', 100, 160, w=130)
    d.node('DES', '1cv8 DESIGNER', 'sw', 380, 80)
    d.node('AGT', '1cv8 /AgentMode\n127.0.0.1:1543', 'svc', 380, 360)
    d.node('CLI', '1cv8c · клиент', 'sw', 380, 240)
    d.frame('HOST', 'раннер и утилиты', ['R', 'DES', 'CLI', 'AGT'])
    d.node('RAG', 'ragent, запущенный руками\n1540 · 1541', 'sw', 850, 240, w=240, h=80)
    d.node('RPH', 'rphost', 'sw', 850, 370)
    d.node('LOG', 'журнал сервера\nу вас перед глазами', 'doc', 1150, 240, w=200)
    d.frame('CON', 'ваша консоль', ['RAG', 'RPH', 'LOG'], kind='neutral')
    d.frame('BOX', 'одна машина', ['R', 'DES', 'CLI', 'AGT', 'RAG', 'RPH', 'LOG', 'DBMS'], kind='box', pad=44, top=52)
    d.node('DBMS', 'PostgreSQL\nlocalhost:5432', 'cyl', 1320, 370, w=90, h=54)
    host_fan(d, [('DES', FAN, 'r:0.25', 0.7), ('CLI', 'процесс на запуск', 'r:0.5', 0.7)])
    d.edge('R', 'AGT', 'поднимает, потом SSH', fa='b', lx=215)
    # Разделённые линии с подписями на горизонтальных участках.
    # Начало и конец каждого пути принадлежат контурам блоков.
    for source, port, label, dashed in [
        ('DES', 'l:0.25', 'TCP 1541', False),
        ('CLI', 'l', 'TCP', False),
        ('AGT', 'l:0.75', 'TCP', True),
    ]:
        start = d.anchor(source, 'r')[:2]
        end = d.anchor('RAG', port)[:2]
        via = [(650, start[1]), (650, end[1])] if start[1] != end[1] else []
        d.edge(source, 'RAG', label, dashed=dashed, fb=port,
               path=d.rounded_path([start, *via, end]), lx=560, ly=start[1])
    d.edge('RAG', 'RPH', fa='b', fb='t')
    d.edge('RAG', 'LOG')
    d.edge('RPH', 'DBMS')
    d.title += ' — через работающий кластер'
    return d
d_console('d-console')

def d_docker_cluster(id, dbms=False):
    if dbms:
        return d_cluster_stopped(id, 'Кластер 1С в контейнере, СУБД в другом контейнере', 'docker')
    d = diagram(id, 'Кластер 1С в контейнере, СУБД в другом контейнере')
    d.node('R', 'v8-runner', 'run', 100, 200, w=130)
    d.node('DES', '1cv8 DESIGNER', 'sw', 380, 100)
    d.node('AGT', '1cv8 /AgentMode\n127.0.0.1:1543', 'svc', 380, 330)
    d.node('CLI', '1cv8c · клиент', 'sw', 380, 220)
    d.node('WP', 'workPath\n.cf, .dt, отчёты', 'doc', 620, 30, w=170)
    d.frame('HOST', 'машина раннера', ['R', 'DES', 'CLI', 'AGT', 'WP'])
    d.node('RAG', 'ragent 1540 · rmngr 1541', 'sw', 900, 220, w=220, h=80)
    d.node('RPH', 'rphost 1560–1591', 'sw', 900, 340)
    d.frame('C1', 'контейнер сервера 1С', ['RAG', 'RPH'], kind='srv')
    d.node('DBMS', 'PostgreSQL\nпорт хоста 5432', 'cyl', 1200, 340, w=120, h=54)
    d.frame('C2', 'контейнер СУБД', ['DBMS'], kind='neutral')
    host_fan(d, [('DES', FAN, 'r:0.25', 0.7), ('CLI', 'процесс на запуск', 'r:0.5', 0.7)])
    d.edge('R', 'AGT', 'поднимает, потом SSH', fa='b', lx=215)
    for edge in d.edges:
        edge['exit_stub'] = edge['entry_stub'] = 30
    sx, sy, _ = d.anchor('DES', 't')
    tx, ty, _ = d.anchor('WP', 'l')
    d.edge('DES', 'WP', fa='t', fb='l', path=d.rounded_path([(sx, sy), (sx, ty), (tx, ty)]))
    # Each network label sits on its own horizontal departure corridor.
    for source, face, label in [('DES', 'l:0.25', 'опубликованные порты'),
                                ('CLI', 'l:0.5', 'TCP'), ('AGT', 'l:0.75', 'TCP')]:
        sx, sy, _ = d.anchor(source, 'r')
        tx, ty, _ = d.anchor('RAG', face)
        points = [(sx, sy), (tx, ty)] if sy == ty else [(sx, sy), (730, sy), (730, ty), (tx, ty)]
        d.edge(source, 'RAG', label, dashed=True, fa='r', fb=face,
               path=d.rounded_path(points), lx=600, ly=sy)
    for source, target, fa, fb, dashed in [('RAG', 'RPH', 'b', 't', False),
                                          ('RPH', 'DBMS', 'r', 'l', True)]:
        sx, sy, _ = d.anchor(source, fa)
        tx, ty, _ = d.anchor(target, fb)
        d.edge(source, target, fa=fa, fb=fb, dashed=dashed, path=d.rounded_path([(sx, sy), (tx, ty)]))
    d.title += ' — через работающий кластер'
    return d

d_docker_cluster('d-docker-cluster')
d_docker_cluster('d-docker-cluster-dbms', dbms=True)

def d_docker_standalone(id):
    d = diagram(id, 'Автономный сервер в контейнере: SSH-шлюз, общий каталог и HTTP на опубликованных портах')
    d.node('R', 'v8-runner', 'run', 100, 160, w=130)
    d.node('WP', 'workPath\nостаётся здесь', 'doc', 100, 330, w=150)
    d.frame('HOST', 'машина раннера', ['R', 'WP'])
    d.node('GATE', 'SSH-шлюз 1543', 'svc', 640, 60)
    d.node('IBSRV', 'ibsrv', 'sw', 960, 160)
    d.node('UD', 'каталог пользователя шлюза\nсмонтирован томом', 'doc', 960, 330, w=250)
    d.node('HTTPS', 'HTTP 8314', 'svc', 960, 60)
    d.node('DB', 'база данных', 'cyl', 1210, 160, w=120, h=54)
    d.frame('C', 'контейнер ibsrv', ['GATE', 'IBSRV', 'UD', 'HTTPS', 'DB'], kind='srv')
    d.node('BR', 'браузер · тонкий клиент', 'svc', 960, -62, w=200)
    d.edge('R', 'WP', fa='b', fb='t')
    d.edge('R', 'GATE', 'SSH: команда строкой, ответ JSON', dashed=True, fa='r:0.15', fb='l:0.5', lx=332)
    d.edge('R', 'UD', 'exchange: dir', path='M165,188 C400,215 560,336 835,336', lx=380, ly=362)
    d.edge('GATE', 'IBSRV', path='M715,70 C800,70 890,140 900,148')
    d.edge('IBSRV', 'UD', fa='b', fb='t')
    d.edge('IBSRV', 'DB')
    d.edge('IBSRV', 'HTTPS', fa='t', fb='b')
    d.edge('BR', 'HTTPS', dashed=True, fa='b', fb='t')
    return d

d_docker_standalone('d-docker-standalone')

def d_web_remote(id):
    d = diagram(id, 'Кластер с готовой веб-публикацией: раннер открывает клиент и не управляет публикацией')
    d.node('R', 'v8-runner', 'run', 100, 220, w=130)
    d.node('D', '1cv8 DESIGNER', 'sw', 500, 100)
    d.node('WP', 'workPath', 'doc', 500, 200, w=160)
    d.node('CLI', 'браузер · тонкий клиент', 'svc', 500, 300, w=230, h=58)
    d.frame('HOST', 'машина раннера', ['R', 'D', 'WP', 'CLI'], extra=((615, 380, 615, 380),))
    d.node('AP', 'Apache · IIS\nмодуль wsap · wsisapi', 'sw', 900, 300, w=210)
    d.node('VRD', 'готовая публикация\ndefault.vrd', 'doc', 900, 460, w=190)
    d.frame('WEB', 'веб-сервер вне управления раннера', ['AP', 'VRD'], kind='neutral')
    d.node('CL', 'ragent 1540\nrmngr 1541', 'sw', 1300, 300, w=180, h=140)
    d.node('RP', 'rphost', 'sw', 1300, 460)
    d.frame('SRV', 'сервер 1С', ['CL', 'RP'], kind='srv')
    def route(a, b, label=None, fa='r', fb='l', via=(), dashed=False, lx=None, ly=None):
        sx, sy, _ = d.anchor(a, fa)
        tx, ty, _ = d.anchor(b, fb)
        d.edge(a, b, label, fa=fa, fb=fb, dashed=dashed,
               path=d.rounded_path([(sx, sy), *via, (tx, ty)]), lx=lx, ly=ly)
    route('R', 'D', FAN, fa='t', via=((100, 100),), lx=270, ly=100)
    route('R', 'CLI', 'процесс на запуск', fa='b', via=((100, 300),), lx=240, ly=300)
    route('D', 'WP', fa='b', fb='t')
    route('D', 'CL', 'Srvr=…;Ref=…, TCP 1541', fb='l:0.2',
          via=((1160, 100), (1160, 258)), dashed=True, lx=900, ly=100)
    route('CLI', 'AP', 'HTTP · HTTPS, web.url', dashed=True, lx=705, ly=300)
    route('AP', 'CL', 'TCP 1541', dashed=True, lx=1100, ly=300)
    route('AP', 'VRD', fa='b', fb='t')
    route('CL', 'RP', fa='b', fb='t')
    return d
d_web_remote('d-web-remote')

def d_web_pair(id, two_infobases=False):
    first, second = ('origin', 'test') if two_infobases else ('demo', 'demo_int')
    title = ('Две кластерные базы и две веб-публикации: origin и test' if two_infobases
             else 'Одна кластерная база и две веб-публикации: demo и demo_int')
    d = diagram(id, title)
    d.node('R', 'v8-runner', 'run', 100, 260, w=130)
    d.node('CLI', 'браузер · тонкий клиент', 'svc', 450, 260, w=230, h=58)
    d.frame('HOST', 'машина раннера', ['R', 'CLI'])
    d.node('PUBLIC', f'публикация /{first}', 'svc', 900, 120, w=210)
    d.node('INTERNAL', f'публикация /{second}', 'svc', 900, 400, w=230)
    d.node('PUBLIC_DIR', f'/var/www/{first}\ndefault.vrd', 'doc', 900, 240, w=210)
    d.node('INTERNAL_DIR', f'/var/www/{second}\ndefault.vrd', 'doc', 900, 520, w=230)
    d.frame('WEB', 'Apache · две публикации', ['PUBLIC', 'INTERNAL', 'PUBLIC_DIR', 'INTERNAL_DIR'], kind='neutral')
    if two_infobases:
        d.node('ORIGIN_IB', 'база origin\nRef=origin', 'sw', 1300, 120, w=210)
        d.node('TEST_IB', 'база test\nRef=test', 'sw', 1300, 400, w=210)
        d.frame('CLUSTER', 'кластер 1С · две базы', ['ORIGIN_IB', 'TEST_IB'], kind='srv')
    else:
        d.node('IB', 'кластер 1С\nодна база Ref=demo', 'sw', 1300, 340, w=210, h=200)
    def route(a, b, label=None, fa='r', fb='l', via=(), dashed=False, lx=None, ly=None):
        sx, sy, _ = d.anchor(a, fa)
        tx, ty, _ = d.anchor(b, fb)
        d.edge(a, b, label, fa=fa, fb=fb, dashed=dashed,
               path=d.rounded_path([(sx, sy), *via, (tx, ty)]), lx=lx, ly=ly)
    route('R', 'CLI', 'процесс на запуск', lx=245, ly=260)
    route('CLI', 'PUBLIC', f'HTTP · HTTPS: {first}', fa='t', via=((450, 120),), dashed=True, lx=665, ly=120)
    route('CLI', 'INTERNAL', f'HTTP · HTTPS: {second}', fa='b', via=((450, 400),), dashed=True, lx=665, ly=400)
    route('PUBLIC', 'PUBLIC_DIR', fa='b', fb='t')
    route('INTERNAL', 'INTERNAL_DIR', fa='b', fb='t')
    if two_infobases:
        route('PUBLIC', 'ORIGIN_IB', 'TCP', dashed=True, lx=1090, ly=120)
        route('INTERNAL', 'TEST_IB', 'TCP', dashed=True, lx=1100, ly=400)
    else:
        route('PUBLIC', 'IB', 'TCP', fb='l:0.2', via=((1140, 120), (1140, 280)), dashed=True, lx=1090, ly=120)
        route('INTERNAL', 'IB', 'TCP', fb='l:0.8', dashed=True, lx=1100, ly=400)
    return d

d_web_pair('d-web-dual')
d_web_pair('d-web-two-infobases', two_infobases=True)

def d_web_docker(id):
    d = diagram(id, 'Веб-сервер в контейнере с общим томом для каталога публикации')
    d.node('R', 'v8-runner', 'run', 100, 150, w=130)
    d.node('WI', 'webinst', 'sw', 380, 80)
    d.node('VOL', 'том с каталогом публикации\nпуть раннера', 'doc', 380, 240, w=250)
    d.frame('HOST', 'машина раннера', ['R', 'WI', 'VOL'])
    d.node('AP', 'Apache\nмодуль wsap', 'sw', 780, 80, w=150)
    d.node('VRDC', 'тот же том\nпуть внутри контейнера', 'doc', 780, 240, w=210)
    d.node('CONF', 'конфиг из образа\nнастроен заранее', 'doc', 1030, 240, w=200)
    d.node('HTTPP', 'HTTP 80 · 443', 'svc', 1030, 80)
    d.frame('C', 'контейнер веб-сервера', ['AP', 'VRDC', 'CONF', 'HTTPP'], kind='neutral')
    d.node('CL', 'ragent · rmngr 1541', 'sw', 1410, 80, w=190)
    d.frame('SRV', 'сервер 1С', ['CL'], kind='srv')
    d.node('BR', 'браузер · тонкий клиент', 'svc', 1410, 240, w=200)
    d.edge('R', 'WI', FAN, fa='r:0.3', t=0.6)
    d.edge('WI', 'VOL', '-dir: пишет default.vrd', fa='b', fb='t', t=0.5)
    d.edge('VOL', 'VRDC', 'один том, два пути', dashed=True, t=0.5)
    d.edge('AP', 'VRDC', fa='b', fb='t')
    d.edge('AP', 'CONF', path='M855,92 C930,92 1030,140 %.1f,%.1f' % d.anchor('CONF', 't')[:2])
    d.edge('AP', 'CL', 'TCP 1541', dashed=True, path='M855,66 C960,20 1200,20 1315,72', lx=1045, ly=28)
    d.edge('AP', 'HTTPP', fa='r:0.5', fb='l')
    d.edge('BR', 'HTTPP', 'HTTP · HTTPS, ws=…', dashed=True, path='M1310,230 C1230,200 1100,150 1080,102', lx=1222)
    return d
d_web_docker('d-web-docker')

PAGES = {
    'index.html': ['map'],
    'architecture.html': ['map', 'layers', 'processes'],
    'sources.html': ['mapping', 'states-pair', 'states-inside'],
    'glossary.html': ['legend'],
    'deployments.html': ['map'],
    'variant-file.html': ['d-file'],
    'variant-cluster.html': ['d-cluster-local', 'd-cluster-remote', 'd-cluster-ha', 'd-console', 'd-cluster-managed', 'd-cluster-local-dbms', 'd-cluster-remote-dbms', 'd-ras', 'd-ras-managed'],
    'variant-standalone.html': ['d-standalone-local', 'd-standalone-remote', 'd-standalone-managed'],
    'variant-containers.html': ['d-docker-cluster', 'd-docker-cluster-dbms', 'd-docker-standalone'],
    'variant-publication.html': ['d-file-web', 'd-cluster-web', 'd-web-remote', 'd-web-docker', 'd-web-two-infobases', 'd-web-dual'],
}

BLOCK = re.compile(r'<!-- diagram:([a-z0-9-]+) -->\n.*?<!-- /diagram:\1 -->', re.S)
OLD = re.compile(r'<pre class="mermaid">\n.*?</pre>|<div class="diagram" role="img"[^>]*>\n<svg.*?</svg>\n</div>', re.S)

def inject(page, ids):
    p = os.path.join(os.path.dirname(os.path.abspath(__file__)), page)
    with open(p, encoding='utf-8') as source:
        s = source.read()
    blocks = [(m.start(), m.end(), m.group(1) if m.re is BLOCK else None) for m in sorted(list(BLOCK.finditer(s)) + list(OLD.finditer(s)), key=lambda m: m.start())]
    # старые блоки внутри новых маркеров не считаем
    flat = []
    for b in blocks:
        if flat and b[0] < flat[-1][1]: continue
        flat.append(b)
    assert len(flat) == len(ids), (page, len(flat), len(ids))
    marked_ids = [id for _, _, id in flat]
    if any(id is not None for id in marked_ids):
        assert all(id is not None for id in marked_ids), (page, 'mixed marked and legacy diagrams')
        assert len(set(marked_ids)) == len(marked_ids), (page, 'duplicate diagram IDs')
        assert set(marked_ids) == set(ids), (page, 'diagram IDs differ from PAGES', marked_ids, ids)
        replacements = marked_ids
    else:
        replacements = ids  # One-time migration of legacy unmarked diagrams.
    out, pos = [], 0
    for (st, en, _), id in zip(flat, replacements):
        out.append(s[pos:st]); out.append('<!-- diagram:%s -->\n%s\n<!-- /diagram:%s -->' % (id, DIAGRAMS[id].render(), id)); pos = en
    out.append(s[pos:])
    with open(p, 'w', encoding='utf-8') as target:
        target.write(''.join(out))
    return len(flat)

if __name__ == '__main__':
    if len(sys.argv) > 2 and sys.argv[1] == '--print':
        print(DIAGRAMS[sys.argv[2]].render()); sys.exit(0)
    for page, ids in PAGES.items():
        print(page, inject(page, ids))
