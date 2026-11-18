#!/usr/bin/env python3
"""Замер #777: исход входа интерактивного клиента 1С и записи журнала регистрации файловой базы.

python3 startup_probe.py --thin <путь к 1cv8c> --thick <путь к 1cv8> --ib <каталог файловой базы> \
    --user <имя> --password <пароль> [--watch 10]

База — копия, в ней пользователь с паролем и ролью с правом тонкого и толстого клиента;
журнал регистрации в последовательном формате (`1Cv8Log/1Cv8.lgf` и `*.lgp`). Скрипт печатает по
строке JSON на случай и добавленные в журнал записи. Пароль в вывод не попадает.
macOS: тонкий клиент — бинарник внутри `1cv8c.app/Contents/MacOS/`, толстый — `1cv8` установки.
"""
import argparse, glob, json, os, signal, subprocess, sys, time


def lgp_state(log):
    return {p: os.path.getsize(p) for p in glob.glob(os.path.join(log, '*.lgp'))}


def new_text(log, before):
    out = []
    for p in sorted(glob.glob(os.path.join(log, '*.lgp'))):
        with open(p, 'rb') as f:
            f.seek(before.get(p, 0))
            out.append(f.read().decode('utf-8', 'replace'))
    return ''.join(out)


def stop(p, graceful=False):
    if p.poll() is not None:
        return
    if os.name == 'nt':
        subprocess.run(['taskkill'] + ([] if graceful else ['/F', '/T']) + ['/PID', str(p.pid)], capture_output=True)
    else:
        p.send_signal(signal.SIGTERM if graceful else signal.SIGKILL)


def run(a, name, client, creds, ib=None, close_at=None):
    ib = ib or a.ib
    log = os.path.join(a.ib, '1Cv8Log')
    exe = a.thin if client == 'thin' else a.thick
    args = [exe, 'ENTERPRISE', '/DisableStartupDialogs', '/IBConnectionString', 'File=' + ib] + creds
    before = lgp_state(log)
    t0 = time.time()
    p = subprocess.Popen(args, stdin=subprocess.DEVNULL)
    rc = t_exit = first_log = None
    while time.time() - t0 < a.watch:
        r = p.poll()
        if r is not None and rc is None:
            rc, t_exit = r, round(time.time() - t0, 1)
        if first_log is None and new_text(log, before).strip():
            first_log = round(time.time() - t0, 1)
        if close_at and time.time() - t0 > close_at:
            stop(p, graceful=True)
            close_at = None
        time.sleep(0.2)
    alive = p.poll() is None
    stop(p)
    p.wait()
    time.sleep(1.5)
    print(json.dumps({'case': name, 'client': client, 'exit': rc, 'exit_at_s': t_exit,
                      'alive_at_end': alive, 'first_log_at_s': first_log}, ensure_ascii=False))
    print(new_text(log, before))
    sys.stdout.flush()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--thin', required=True)
    ap.add_argument('--thick', required=True)
    ap.add_argument('--ib', required=True)
    ap.add_argument('--user', required=True)
    ap.add_argument('--password', required=True)
    ap.add_argument('--watch', type=float, default=10)
    a = ap.parse_args()
    u, pw = a.user, a.password
    cases = [('ok', ['/N', u, '/P', pw], None), ('wrong_password', ['/N', u, '/P', pw + 'x'], None),
             ('no_password', ['/N', u], None), ('unknown_user', ['/N', 'NoSuchUser777', '/P', 'x'], None),
             ('no_creds', [], None), ('graceful_close', ['/N', u, '/P', pw], 5)]
    for client in ('thin', 'thick'):
        for name, creds, close_at in cases:
            run(a, name, client, creds, close_at=close_at)
        run(a, 'broken_base', client, ['/N', u, '/P', pw], ib=a.ib + '_missing')
    print(open(os.path.join(a.ib, '1Cv8Log', '1Cv8.lgf'), encoding='utf-8-sig', errors='replace').read())


if __name__ == '__main__':
    main()
