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
        self.nodes[id] = dict(id=id, text=text, kind=kind, x=x, y=y, w=w, h=h)
        return id

    def frame(self, id, title, members, kind='host', pad=18, top=26, extra=()):
        self.frames.append(dict(id=id, title=title, members=members, kind=kind, pad=pad, top=top, extra=extra))

    def edge(self, a, b, label=None, dashed=False, fa='r', fb='l', t=0.5, dx=0, dy=0, path=None, arrow=True, lx=None, ly=None):
        self.edges.append(dict(a=a, b=b, label=label, dashed=dashed, fa=fa, fb=fb, t=t, dx=dx, dy=dy, path=path, arrow=arrow, lx=lx, ly=ly))

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
    def path_points(path):
        """точки пути с шагом по кривым — чтобы рамка картинки учитывала линии, а не только узлы"""
        pts, cur = [], None
        for cmd, args in re.findall(r'([MLCQ])\s*([^MLCQ]*)', path):
            v = [float(x) for x in re.findall(r'-?[\d.]+', args)]
            if cmd in ('M', 'L'):
                for i in range(0, len(v), 2): cur = (v[i], v[i + 1]); pts.append(cur)
            elif cmd == 'C':
                for i in range(0, len(v), 6):
                    p = [cur, (v[i], v[i + 1]), (v[i + 2], v[i + 3]), (v[i + 4], v[i + 5])]
                    pts += [Diagram.bezier(*p, k / 10) for k in range(11)]; cur = p[3]
            elif cmd == 'Q':
                for i in range(0, len(v), 4):
                    c, e = (v[i], v[i + 1]), (v[i + 2], v[i + 3])
                    pts += [((1 - t) ** 2 * cur[0] + 2 * (1 - t) * t * c[0] + t * t * e[0], (1 - t) ** 2 * cur[1] + 2 * (1 - t) * t * c[1] + t * t * e[1]) for t in [k / 10 for k in range(11)]]
                    cur = e
        return pts

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
            m = re.match(r'M\s*([\d.-]+),([\d.-]+)\s*C\s*([\d.-]+),([\d.-]+)\s+([\d.-]+),([\d.-]+)\s+([\d.-]+),([\d.-]+)', e['path'])
            if m:
                v = [float(x) for x in m.groups()]
                p = [(v[0], v[1]), (v[2], v[3]), (v[4], v[5]), (v[6], v[7])]
                if e['lx'] is not None: return e['path'], self.at_x(p, e['lx'])
                return e['path'], self.bezier(*p, e['t'])
            m = re.match(r'M\s*([\d.-]+),([\d.-]+)\s*L\s*([\d.-]+),([\d.-]+)', e['path'])
            v = [float(x) for x in m.groups()]
            return e['path'], (v[0] + (v[2] - v[0]) * e['t'], v[1] + (v[3] - v[1]) * e['t'])
        (x1, y1, d1), (x2, y2, d2) = self.anchor(e['a'], e['fa']), self.anchor(e['b'], e['fb'])
        dist = max(abs(x2 - x1), abs(y2 - y1))
        k = max(40, dist * 0.45)
        c1 = (x1 + d1[0] * k, y1 + d1[1] * k)
        c2 = (x2 + d2[0] * k, y2 + d2[1] * k)
        path = 'M%.1f,%.1f C%.1f,%.1f %.1f,%.1f %.1f,%.1f' % (x1, y1, c1[0], c1[1], c2[0], c2[1], x2, y2)
        if e['lx'] is not None and e['ly'] is not None: return path, (e['lx'], e['ly'])
        if e['lx'] is not None: return path, self.at_x([(x1, y1), c1, c2, (x2, y2)], e['lx'])
        return path, self.bezier((x1, y1), c1, c2, (x2, y2), e['t'])

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
    d = diagram(id, 'Кто с кем говорит: раннер запускает утилиты у себя и достаёт цель файлом, протоколом платформы и SSH')
    d.node('R', 'v8-runner\nCLI · MCP', 'run', 100, 300, w=140)
    d.node('EDT', '1cedtcli', 'sw', 440, 60)
    d.node('V8', '1cv8 DESIGNER\n1cv8 /AgentMode 1543', 'sw', 440, 170, w=180)
    d.node('IBC', 'ibcmd', 'sw', 440, 290)
    d.node('RAC', 'rac', 'sw', 440, 400)
    d.node('WEB', 'webinst', 'sw', 440, 510)
    d.frame('HOST', 'машина раннера', ['R', 'EDT', 'V8', 'IBC', 'RAC', 'WEB'])
    d.node('SRC', 'исходники\nXML, проект EDT', 'doc', 740, 60)
    d.node('FDB', 'файловая база\nкаталог с 1Cv8.1CD', 'doc', 740, 178, w=200)
    d.node('GATE', 'автономный сервер ibsrv\nпрямой шлюз · SSH-шлюз 1543', 'svc', 1040, 60, w=280)
    d.node('CLU', 'кластер 1С\nragent · rmngr · rphost', 'sw', 1040, 178, w=200)
    d.node('DBMS', 'СУБД', 'cyl', 1040, 300, w=90, h=54)
    d.node('RAS', 'RAS 1545', 'svc', 1250, 178)
    d.node('WS', 'Apache · IIS', 'svc', 740, 510)
    host_fan(d, [('EDT', 'одна живая сессия', 'r:0.05', 0), ('V8', 'процесс на операцию', 'r:0.22', 0, 'l:0.18'), ('V8', 'поднимает, потом SSH', 'r:0.5', 0, 'l:0.85'),
                 ('IBC', 'процесс на операцию', 'r:0.68', 0), ('RAC', 'процесс на операцию', 'r:0.85', 0), ('WEB', 'процесс на операцию', 'r:0.97', 0)])
    d.edge('R', 'GATE', 'SSH · SFTP', dashed=True, path='M150,271 L150,-40 Q150,-70 180,-70 L1010,-70 Q1040,-70 1040,-40 L1040,31', lx=560, ly=-70)
    d.edge('R', 'RAS', 'поднимает, если cluster.ras не объявлен', path='M100,329 L100,600 Q100,630 130,630 L1350,630 Q1380,630 1380,600 L1380,208 Q1380,178 1350,178 L1304,178', lx=1100, ly=630)
    d.edge('EDT', 'SRC', fb='l:0.4')
    d.edge('V8', 'SRC', fa='r:0.15', fb='l:0.75')
    d.edge('V8', 'FDB', 'файл', fa='r:0.6', fb='l:0.5', t=0.5)
    d.edge('V8', 'CLU', 'TCP 1541', dashed=True, path='M530,167 C640,105 800,105 940,166', t=0.5)
    d.edge('V8', 'GATE', 'TCP: прямой шлюз', dashed=True, path='M512,141 C512,-30 800,-40 900,60', t=0.5)
    d.edge('IBC', 'FDB', '--db-path', fa='r:0.3', fb='l:0.85', lx=594)
    d.edge('IBC', 'DBMS', '--dbms: прямо в СУБД', dashed=True, fa='r:0.7', fb='l', lx=700)
    d.edge('RAC', 'RAS', 'TCP 1545: сеансы своей базы', dashed=True, path='M488,400 C800,400 1250,400 1250,200', lx=700)
    d.edge('RAS', 'CLU', fa='l', fb='r')
    d.edge('CLU', 'DBMS', fa='b', fb='t')
    d.edge('WEB', 'WS', 'публикация', lx=609)
    return d

general_map('map')

def layers(id):
    d = diagram(id, 'Слои раннера: адаптеры транспорта, оркестрация, общие слои и платформа')
    d.node('CLI', 'cli\nаргументы, текст и JSON', 'sw', 150, 60)
    d.node('MCP', 'mcp\n8 инструментов, stdio и HTTP', 'sw', 430, 60)
    d.frame('ad', 'Адаптеры транспорта', ['CLI', 'MCP'])
    d.node('UC', 'use_cases — оркестрация\nзамок, пайплайн', 'run', 290, 190)
    d.node('CFG', 'config', 'sw', 90, 320)
    d.node('DOM', 'domain', 'sw', 230, 320)
    d.node('CD', 'change_detection', 'sw', 400, 320)
    d.node('PARS', 'parsers', 'sw', 570, 320)
    d.frame('sh', 'Общее', ['CFG', 'DOM', 'CD', 'PARS'])
    d.node('PLAT', 'platform — вызов утилит', 'sw', 780, 190)
    d.node('SUP', 'support — файлы, staging, журналы', 'sw', 780, 320)
    d.edge('CLI', 'UC', fa='b', fb='t:0.3')
    d.edge('MCP', 'UC', fa='b', fb='t:0.7')
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
    d.node('R', 'раннер\nCLI или MCP-сервер', 'run', 450, 270, w=160)
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
    d.node('IB', 'каталог базы\n1Cv8.1CD', 'doc', 720, 150, w=170)
    d.node('WP', 'workPath\nжурналы, хеши, .cf, .dt', 'doc', 720, 420, w=210)
    d.frame('HOST', 'машина раннера', ['R', 'DES', 'IBC', 'AGT', 'CLI', 'IB', 'WP'])
    host_fan(d, [('DES', FAN, 'r:0.15', 0.7), ('IBC', FAN, 'r:0.4', 0.7), ('AGT', 'поднимает, потом SSH', 'r:0.65', 0.5), ('CLI', 'процесс на запуск', 'r:0.9', 0.7)])
    d.edge('DES', 'IB', 'файл, монопольно', fa='r:0.4', fb='l:0.2', t=0.5)
    d.edge('IBC', 'IB', '--db-path', fb='l:0.5')
    d.edge('AGT', 'IB', 'файл', fa='r:0.3', fb='l:0.8', t=0.5)
    d.edge('CLI', 'IB', 'файл', fb='b:0.3', t=0.5)
    d.edge('DES', 'WP', path='M445,72 C560,72 560,420 615,420')
    return d
d_file('d-file')

def d_file_web(id):
    d = diagram(id, 'Файловая база и веб-сервер на машине раннера')
    d.node('R', 'v8-runner', 'run', 100, 220, w=130)
    d.node('WEBI', 'webinst', 'sw', 380, 80)
    d.node('DES', '1cv8 DESIGNER', 'sw', 380, 220)
    d.node('CLI', '1cv8c · клиент', 'sw', 380, 340)
    d.node('WS', 'Apache · IIS\nмодуль wsap · wsisapi', 'svc', 740, 80, w=230)
    d.node('PUB', 'каталог публикации\ndefault.vrd', 'doc', 740, 220, w=190)
    d.node('IB', 'каталог базы\n1Cv8.1CD', 'doc', 740, 360, w=170)
    d.frame('HOST', 'машина раннера', ['R', 'WEBI', 'DES', 'CLI', 'WS', 'PUB', 'IB'])
    d.node('BR', 'браузер · тонкий клиент', 'svc', 1150, 80)
    host_fan(d, [('WEBI', FAN, 'r:0.2', 0.7), ('DES', FAN, 'r:0.5', 0.6), ('CLI', 'процесс на запуск', 'r:0.8', 0.7)])
    d.edge('CLI', 'IB', 'File=…, файл', fb='l:0.85', t=0.5)
    d.edge('WEBI', 'PUB', '-dir, -wsdir, -connstr File=…', path='M430,92 C520,92 560,208 645,208', lx=560, ly=150)
    d.edge('WEBI', 'WS', '-confpath: правка конфига', fa='r:0.5', fb='l:0.5', t=0.5)
    d.edge('WS', 'IB', 'файл', path='M625,90 C570,90 570,336 655,336', lx=575, ly=215)
    d.edge('WS', 'PUB', fa='b', fb='t')
    d.edge('DES', 'IB', 'файл, монопольно', fa='r:0.5', fb='l:0.6', t=0.5)
    d.edge('BR', 'WS', 'HTTP · HTTPS, ws=…', dashed=True, fa='l', fb='r', lx=956)
    return d

d_file_web('d-file-web')

def d_cluster_local(id):
    d = diagram(id, 'Кластер на машине раннера: раннер, утилиты и сервер 1С на одной машине')
    d.node('R', 'v8-runner', 'run', 80, 220, w=130)
    d.node('DES', '1cv8 DESIGNER', 'sw', 380, 80)
    d.node('IBC', 'ibcmd', 'sw', 380, 170)
    d.node('AGT', '1cv8 /AgentMode\n127.0.0.1:1543', 'svc', 380, 265)
    d.node('CLI', '1cv8c · клиент', 'sw', 380, 360)
    d.node('WP', 'workPath\n.cf, .dt, журналы', 'doc', 620, 80, w=170)
    d.frame('HOST', 'раннер и утилиты', ['R', 'DES', 'IBC', 'AGT', 'CLI', 'WP'])
    d.node('RAG', 'ragent 1540\nrmngr 1541', 'sw', 820, 170)
    d.node('RPH', 'rphost 1560–1591', 'sw', 820, 290)
    d.frame('SRV', 'сервер 1С', ['RAG', 'RPH'], kind='srv')
    d.frame('BOX', 'одна машина', ['R', 'DES', 'IBC', 'AGT', 'CLI', 'WP', 'RAG', 'RPH'], kind='box', pad=44, top=52)
    d.node('DBMS', 'СУБД', 'cyl', 1140, 290, w=90, h=54)
    host_fan(d, [('DES', FAN, 'r:0.15', 0.7), ('IBC', FAN, 'r:0.4', 0.7), ('AGT', 'поднимает, потом SSH', 'r:0.65', 0.5), ('CLI', 'процесс на запуск', 'r:0.9', 0.7)])
    d.edge('DES', 'WP', fa='r:0.3', fb='l')
    d.edge('DES', 'RAG', 'Srvr=…;Ref=…, TCP', path='M445,90 C560,140 690,150 764,160', lx=600, ly=128)
    d.edge('CLI', 'RAG', 'TCP', fb='l:0.9', t=0.6)
    d.edge('AGT', 'RAG', 'TCP', fa='r:0.4', fb='l:0.6', t=0.6)
    d.edge('IBC', 'DBMS', '--dbms, --database-server', path='M428,180 C560,180 620,400 840,400 C980,400 1080,380 1140,318', lx=840, ly=400)
    d.edge('RAG', 'RPH', fa='b', fb='t')
    d.edge('RPH', 'DBMS')
    return d

d_cluster_local('d-cluster-local')

def d_cluster_remote(id):
    d = diagram(id, 'Кластер на другой машине с несколькими rphost')
    d.node('R', 'v8-runner', 'run', 100, 200, w=130)
    d.node('DES', '1cv8 DESIGNER', 'sw', 380, 100)
    d.node('CLI', '1cv8c · клиент', 'sw', 380, 220)
    d.node('WP', 'workPath\n.cf, .dt, отчёты тестов', 'doc', 620, 100, w=210)
    d.frame('HOST', 'машина раннера', ['R', 'DES', 'CLI', 'WP'])
    d.node('RAG', 'ragent 1540 · rmngr 1541', 'sw', 960, 100, w=220)
    d.node('RP1', 'rphost', 'sw', 840, 220)
    d.node('RP2', 'rphost', 'sw', 960, 220)
    d.node('RP3', 'rphost', 'sw', 1080, 220)
    d.node('LOG', 'журнал регистрации,\nтехнологический журнал', 'docoff', 960, 350, w=220)
    d.frame('SRV', 'сервер приложений 1С', ['RAG', 'RP1', 'RP2', 'RP3', 'LOG'], kind='srv')
    d.node('DBMS', 'СУБД\n5432 · 1433', 'cyl', 1320, 220, w=110, h=60)
    host_fan(d, [('DES', FAN, 'r:0.25', 0.7), ('CLI', 'процесс на запуск', 'r:0.75', 0.7)])
    d.edge('DES', 'WP', fa='r:0.3', fb='l')
    d.edge('DES', 'RAG', 'TCP 1541, дальше 1560–1591', dashed=True, path='M436,108 C560,160 720,160 850,92', lx=640, ly=160)
    d.edge('CLI', 'RAG', 'TCP', dashed=True, fb='l:0.8', lx=796)
    d.edge('RAG', 'RP1', fa='b:0.2', fb='t')
    d.edge('RAG', 'RP2', fa='b:0.5', fb='t')
    d.edge('RAG', 'RP3', fa='b:0.8', fb='t')
    d.edge('RP1', 'DBMS', path='M840,242 C840,300 1140,300 1265,238')
    d.edge('RP2', 'DBMS', path='M960,242 C960,285 1150,290 1265,232')
    d.edge('RP3', 'DBMS', fb='l:0.3')
    d.edge('RP1', 'LOG', fa='b:0.4', fb='t:0.2')
    return d

d_cluster_remote('d-cluster-remote')

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
    d = diagram(id, 'Кластер и веб-сервер на машине раннера')
    d.node('R', 'v8-runner', 'run', 100, 200, w=130)
    d.node('WEBI', 'webinst', 'sw', 380, 80)
    d.node('CLI', '1cv8c · клиент', 'sw', 380, 330)
    d.node('PUB', 'каталог публикации\ndefault.vrd', 'doc', 740, 80, w=190)
    d.node('WS', 'Apache · IIS\nмодуль wsap · wsisapi', 'svc', 740, 210, w=230)
    d.frame('HOST', 'машина раннера', ['R', 'WEBI', 'CLI', 'WS', 'PUB'])
    d.node('RAG', 'ragent · rmngr 1541', 'sw', 1100, 330, w=190)
    d.node('RPH', 'rphost', 'sw', 1320, 330)
    d.frame('SRV', 'сервер 1С', ['RAG', 'RPH'], kind='srv')
    d.node('BR', 'браузер · тонкий клиент', 'svc', 1170, 80)
    host_fan(d, [('WEBI', FAN, 'r:0.25', 0.6), ('CLI', 'процесс на запуск', 'r:0.75', 0.6)])
    d.edge('WEBI', 'PUB', '-dir, -wsdir,\n-connstr Srvr=…;Ref=…', t=0.5)
    d.edge('WEBI', 'WS', '-confpath: правка конфига', fa='r:0.7', fb='l:0.5', t=0.5)
    d.edge('WS', 'PUB', fa='t', fb='b')
    d.edge('CLI', 'RAG', 'Srvr=…;Ref=…, TCP 1541', dashed=True, t=0.5)
    d.edge('WS', 'RAG', 'TCP 1541', dashed=True, path='M809,239 C900,239 1140,250 1140,308', t=0.4)
    d.edge('BR', 'WS', 'HTTP · HTTPS, ws=…', dashed=True, fa='l', fb='r:0.4', t=0.5)
    d.edge('RAG', 'RPH')
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
    d.node('IBSRV', 'ibsrv\nподнят вами', 'sw', 1060, 190)
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

def d_console(id):
    d = diagram(id, 'Кластер поднят из консоли, а не службой')
    d.node('R', 'v8-runner', 'run', 100, 160, w=130)
    d.node('DES', '1cv8 DESIGNER', 'sw', 380, 80)
    d.node('CLI', '1cv8c · клиент', 'sw', 380, 240)
    d.frame('HOST', 'раннер и утилиты', ['R', 'DES', 'CLI'])
    d.node('RAG', 'ragent, запущенный руками\n1540 · 1541', 'sw', 780, 120, w=240)
    d.node('RPH', 'rphost', 'sw', 780, 250)
    d.node('LOG', 'журнал сервера\nу вас перед глазами', 'doc', 1050, 120, w=200)
    d.frame('CON', 'ваша консоль', ['RAG', 'RPH', 'LOG'], kind='neutral')
    d.frame('BOX', 'одна машина', ['R', 'DES', 'CLI', 'RAG', 'RPH', 'LOG'], kind='box', pad=44, top=52)
    d.node('DBMS', 'СУБД', 'cyl', 1320, 250, w=90, h=54)
    host_fan(d, [('DES', FAN, 'r:0.25', 0.7), ('CLI', 'процесс на запуск', 'r:0.75', 0.7)])
    d.edge('DES', 'RAG', 'Srvr=localhost:1541;Ref=…', fb='l:0.4', t=0.5)
    d.edge('CLI', 'RAG', 'TCP', fb='l:0.85', t=0.6)
    d.edge('RAG', 'RPH', fa='b', fb='t')
    d.edge('RAG', 'LOG')
    d.edge('RPH', 'DBMS')
    return d
d_console('d-console')

def d_docker_cluster(id):
    d = diagram(id, 'Кластер 1С в контейнере, СУБД в другом контейнере')
    d.node('R', 'v8-runner', 'run', 100, 200, w=130)
    d.node('DES', '1cv8 DESIGNER', 'sw', 380, 100)
    d.node('CLI', '1cv8c · клиент', 'sw', 380, 220)
    d.node('WP', 'workPath\n.cf, .dt, отчёты', 'doc', 620, 100, w=170)
    d.frame('HOST', 'машина раннера', ['R', 'DES', 'CLI', 'WP'])
    d.node('RAG', 'ragent 1540 · rmngr 1541', 'sw', 900, 120, w=220)
    d.node('RPH', 'rphost 1560–1591', 'sw', 900, 240)
    d.frame('C1', 'контейнер сервера 1С', ['RAG', 'RPH'], kind='srv')
    d.node('DBMS', 'база данных', 'cyl', 1200, 240, w=120, h=54)
    d.frame('C2', 'контейнер СУБД', ['DBMS'], kind='neutral')
    host_fan(d, [('DES', FAN, 'r:0.25', 0.7), ('CLI', 'процесс на запуск', 'r:0.75', 0.7)])
    d.edge('DES', 'WP', fa='r:0.3', fb='l')
    d.edge('DES', 'RAG', 'опубликованные порты', dashed=True, path='M436,108 C560,160 700,160 790,112', lx=640, ly=160)
    d.edge('CLI', 'RAG', 'TCP', dashed=True, fb='l:0.8', lx=756)
    d.edge('RAG', 'RPH', fa='b', fb='t')
    d.edge('RPH', 'DBMS', dashed=True)
    return d

d_docker_cluster('d-docker-cluster')

def d_docker_standalone(id):
    d = diagram(id, 'Автономный сервер в контейнере: SSH-шлюз, прямой шлюз и HTTP на опубликованных портах')
    d.node('R', 'v8-runner', 'run', 100, 160, w=130)
    d.node('WP', 'workPath\nостаётся здесь', 'doc', 100, 330, w=150)
    d.frame('HOST', 'машина раннера', ['R', 'WP'])
    d.node('GATE', 'SSH-шлюз 1543', 'svc', 640, 60)
    d.node('DGATE', 'прямой шлюз — если порт опубликован', 'svc', 640, 160, w=300)
    d.node('IBSRV', 'ibsrv', 'sw', 960, 160)
    d.node('UD', 'каталог пользователя шлюза\nсмонтирован томом', 'doc', 960, 330, w=250)
    d.node('HTTPS', 'HTTP 8314', 'svc', 960, 60)
    d.node('DB', 'база данных', 'cyl', 1210, 160, w=120, h=54)
    d.frame('C', 'контейнер ibsrv', ['GATE', 'DGATE', 'IBSRV', 'UD', 'HTTPS', 'DB'], kind='srv')
    d.node('BR', 'браузер · тонкий клиент', 'svc', 960, -62, w=200)
    d.edge('R', 'WP', fa='b', fb='t')
    d.edge('R', 'GATE', 'SSH: команда строкой, ответ JSON', dashed=True, fa='r:0.15', fb='l:0.5', lx=332)
    d.edge('R', 'DGATE', '1cv8 DESIGNER: TCP', dashed=True, fa='r:0.4', fb='l:0.5', lx=330)
    d.edge('R', 'UD', 'exchange: sftp', dashed=True, path='M165,178 C400,178 560,300 835,300', lx=380)
    d.edge('R', 'UD', 'exchange: dir — если том виден раннеру', path='M165,188 C400,215 560,336 835,336', lx=640, ly=362)
    d.edge('GATE', 'IBSRV', path='M715,70 C800,70 890,140 900,148')
    d.edge('DGATE', 'IBSRV')
    d.edge('IBSRV', 'UD', fa='b', fb='t')
    d.edge('IBSRV', 'DB')
    d.edge('IBSRV', 'HTTPS', fa='t', fb='b')
    d.edge('BR', 'HTTPS', dashed=True, fa='b', fb='t')
    return d

d_docker_standalone('d-docker-standalone')

def d_web_remote(id):
    d = diagram(id, 'Веб-сервер на отдельной машине: раннер публиковать не может')
    d.node('R', 'v8-runner', 'run', 100, 200, w=130)
    d.node('D', '1cv8 DESIGNER', 'sw', 380, 80)
    d.node('WI', 'webinst\nпубликовать нечего', 'sw', 380, 200, w=180)
    d.node('WP', 'workPath', 'doc', 380, 320, w=130)
    d.frame('HOST', 'машина раннера', ['R', 'D', 'WI', 'WP'])
    d.node('AP', 'Apache · IIS\nмодуль wsap · wsisapi', 'sw', 780, 130, w=230)
    d.node('VRD', 'каталог публикации\ndefault.vrd', 'doc', 780, 260, w=190)
    d.node('CONF', 'конфиг веб-сервера', 'doc', 1010, 260, w=180)
    d.frame('WEB', 'машина веб-сервера', ['AP', 'VRD', 'CONF'], kind='neutral')
    d.node('CL', 'ragent · rmngr 1541', 'sw', 1300, 130, w=190)
    d.node('RP', 'rphost', 'sw', 1300, 250)
    d.frame('SRV', 'сервер 1С', ['CL', 'RP'], kind='srv')
    d.node('BR', 'браузер · тонкий клиент', 'svc', 252, 420, w=200)
    host_fan(d, [('D', None, 'r:0.2', 0.5), ('WI', None, 'r:0.5', 0.5), ('WP', None, 'r:0.8', 0.5)])
    d.edge('D', 'CL', 'Srvr=…;Ref=…, TCP 1541', dashed=True, path='M444,72 C600,10 1050,10 1205,120', lx=830, ly=28)
    d.edge('CL', 'RP', fa='b', fb='t')
    d.edge('AP', 'VRD', fa='b:0.4', fb='t')
    d.edge('AP', 'CONF', path='M870,150 C930,160 1000,180 1010,230')
    d.edge('AP', 'CL', 'TCP 1541', dashed=True, fa='r:0.4', fb='l:0.6', t=0.5)
    d.edge('BR', 'AP', 'HTTP · HTTPS, ws=…', dashed=True, path='M352,420 C540,420 560,130 665,130', lx=565, ly=232)
    return d
d_web_remote('d-web-remote')

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
    d.edge('AP', 'CONF', path='M855,92 C930,92 1030,140 1030,210')
    d.edge('AP', 'CL', 'TCP 1541', dashed=True, path='M855,66 C960,20 1200,20 1315,72', lx=1045, ly=28)
    d.edge('AP', 'HTTPP', fa='r:0.5', fb='l')
    d.edge('BR', 'HTTPP', 'HTTP · HTTPS, ws=…', dashed=True, path='M1310,230 C1230,200 1100,150 1080,102', lx=1222)
    return d
d_web_docker('d-web-docker')

PAGES = {
    'architecture.html': ['map', 'layers', 'processes'],
    'sources.html': ['mapping', 'states-pair', 'states-inside'],
    'deployments.html': ['legend', 'map', 'd-file', 'd-file-web', 'd-cluster-local', 'd-cluster-remote', 'd-ras', 'd-ras-managed', 'd-cluster-web', 'd-standalone-local', 'd-standalone-remote', 'd-console', 'd-docker-cluster', 'd-docker-standalone', 'd-web-remote', 'd-web-docker'],
}

BLOCK = re.compile(r'<!-- diagram:([a-z0-9-]+) -->\n.*?<!-- /diagram:\1 -->', re.S)
OLD = re.compile(r'<pre class="mermaid">\n.*?</pre>|<div class="diagram" role="img"[^>]*>\n<svg.*?</svg>\n</div>', re.S)

def inject(page, ids):
    p = os.path.join(os.path.dirname(os.path.abspath(__file__)), page)
    s = open(p, encoding='utf-8').read()
    blocks = [(m.start(), m.end(), m.group(1) if m.re is BLOCK else None) for m in sorted(list(BLOCK.finditer(s)) + list(OLD.finditer(s)), key=lambda m: m.start())]
    # старые блоки внутри новых маркеров не считаем
    flat = []
    for b in blocks:
        if flat and b[0] < flat[-1][1]: continue
        flat.append(b)
    assert len(flat) == len(ids), (page, len(flat), len(ids))
    out, pos = [], 0
    for (st, en, _), id in zip(flat, ids):
        out.append(s[pos:st]); out.append('<!-- diagram:%s -->\n%s\n<!-- /diagram:%s -->' % (id, DIAGRAMS[id].render(), id)); pos = en
    out.append(s[pos:])
    open(p, 'w', encoding='utf-8').write(''.join(out))
    return len(flat)

if __name__ == '__main__':
    if len(sys.argv) > 2 and sys.argv[1] == '--print':
        print(DIAGRAMS[sys.argv[2]].render()); sys.exit(0)
    for page, ids in PAGES.items():
        print(page, inject(page, ids))
