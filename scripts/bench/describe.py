"""Stands and scenarios as data: `stands/<name>.toml` and `scenarios.toml`.

A stand is either created by `bench.py setup` (`[create]`: a cf or dt template) under
`<stands_root>/bench-<name>`, or an existing one (`[existing]`: project and infobase paths) that
the script only uses. Both get a work directory `<stands_root>/bench-<name>` for staging, results
and traces. A scenario names its kind (the code), a module set of the stand (the data) and the
executors that can do it.
"""
import os
import tomllib
from dataclasses import dataclass, field

HERE = os.path.dirname(os.path.abspath(__file__))
STANDS_DIR = os.path.join(HERE, "stands")
SCENARIOS_FILE = os.path.join(HERE, "scenarios.toml")

KINDS = ("push", "pull", "status", "pull-none", "push-after-pull", "pull-full")
# Kinds that change no module of a set: the scenario names no `modules`.
NO_MODULES = {"status", "pull-none", "pull-full"}
# runner-016: another runner binary (`--runner-016`, release 0.16) on its own project
# `<work>/project-016` over the same base, so the two runners never share memory or files.
EXECUTORS = ("runner", "runner-live", "runner-016", "agent", "batch", "ibcmd")
RUNNERS = ("runner", "runner-live", "runner-016")
# What each executor can do; a scenario may list only these. `push-after-pull` is the runner's
# own sequence (its memory after a pull); without memory there is nothing to compare.
CAN = {
    "runner": set(KINDS),
    "runner-live": set(KINDS),
    "runner-016": set(KINDS),
    "agent": {"push", "pull", "status", "pull-none", "pull-full"},
    "batch": {"push", "pull", "status", "pull-none", "pull-full"},
    "ibcmd": {"push", "status", "pull-full"},
}
CREATED_MARK = "created-by: scripts/bench"
DEFAULT_STANDS_ROOT = r"E:\stands\v8-runner"
DEFAULT_PLATFORM_ROOT = r"C:\Program Files\1cv8"


class DescriptionError(ValueError):
    pass


@dataclass
class Stand:
    name: str
    title: str
    platform: str
    work: str                 # bench-<name>: staging, results, traces, OWNER.md
    project: str
    infobase: str
    created: bool             # True: `setup` creates it under `work`
    template_cf: str = ""
    template_dt: str = ""
    disable_support: bool = False
    source_set: str = "main"
    source_path: str = "src/main"
    user: str = ""
    password: str = ""
    modules: dict = field(default_factory=dict)
    platform_root: str = DEFAULT_PLATFORM_ROOT

    @property
    def bin(self):
        return os.path.join(self.platform_root, self.platform, "bin")

    @property
    def v8(self):
        return os.path.join(self.bin, "1cv8.exe")

    @property
    def ibcmd(self):
        return os.path.join(self.bin, "ibcmd.exe")

    @property
    def config(self):
        return os.path.join(self.project, "v8project.yaml")

    @property
    def src(self):
        return os.path.join(self.project, *self.source_path.split("/"))

    @property
    def project_016(self):
        """The project of the runner-016 executor: its own tree and memory over the same base."""
        return os.path.join(self.work, "project-016")


@dataclass
class Scenario:
    name: str
    title: str
    kind: str
    modules: str              # module set name in the stand; "" for NO_MODULES kinds
    executors: list
    runner_args: list = field(default_factory=list)


def _require(table, key, where):
    if key not in table:
        raise DescriptionError(f"{where}: no `{key}`")
    return table[key]


def stand_names():
    return sorted(f[:-5] for f in os.listdir(STANDS_DIR) if f.endswith(".toml"))


def load_stand(name, stands_root=None, platform_root=None):
    path = os.path.join(STANDS_DIR, f"{name}.toml")
    if name not in stand_names():
        raise DescriptionError(f"no stand {name!r}; known: {', '.join(stand_names())}")
    with open(path, "rb") as f:
        data = tomllib.load(f)
    where = f"stands/{name}.toml"
    root = stands_root or os.environ.get("V8_BENCH_STANDS_ROOT") or data.get("stands_root") or DEFAULT_STANDS_ROOT
    work = os.path.join(root, f"bench-{name}")
    create, existing = data.get("create"), data.get("existing")
    if bool(create) == bool(existing):
        raise DescriptionError(f"{where}: exactly one of [create] and [existing]")
    modules = data.get("modules") or {}
    for set_name, files in modules.items():
        if not isinstance(files, list) or not files or not all(isinstance(p, str) and p.endswith(".bsl") for p in files):
            raise DescriptionError(f"{where}: module set {set_name!r} must be a non-empty list of .bsl paths")
    common = dict(
        name=name, title=data.get("title", name), platform=_require(data, "platform", where), work=work,
        source_set=data.get("source_set", "main"), source_path=data.get("source_path", "src/main"),
        user=data.get("user", ""), password=data.get("password", ""), modules=modules,
        platform_root=platform_root or os.environ.get("V8_BENCH_PLATFORM_ROOT") or DEFAULT_PLATFORM_ROOT)
    if create:
        cf, dt = create.get("cf", ""), create.get("dt", "")
        if bool(cf) == bool(dt):
            raise DescriptionError(f"{where}: [create] needs exactly one of cf and dt")
        return Stand(project=os.path.join(work, "project"), infobase=os.path.join(work, "ib"), created=True,
                     template_cf=cf, template_dt=dt, disable_support=bool(create.get("disable_support")), **common)
    return Stand(project=_require(existing, "project", where), infobase=_require(existing, "infobase", where),
                 created=False, **common)


def load_scenarios(path=SCENARIOS_FILE):
    with open(path, "rb") as f:
        data = tomllib.load(f)
    scenarios = {}
    for name, table in data.items():
        where = f"scenarios.toml [{name}]"
        kind = _require(table, "kind", where)
        if kind not in KINDS:
            raise DescriptionError(f"{where}: kind {kind!r} is not one of {KINDS}")
        modules = table.get("modules", "")
        if (kind in NO_MODULES) == bool(modules):
            raise DescriptionError(f"{where}: {f'{kind} takes no modules' if modules else 'needs `modules`'}")
        executors = table.get("executors") or list(EXECUTORS)
        for executor in executors:
            if executor not in CAN:
                raise DescriptionError(f"{where}: unknown executor {executor!r}")
            if kind not in CAN[executor]:
                raise DescriptionError(f"{where}: executor {executor!r} cannot do {kind}")
        scenarios[name] = Scenario(name=name, title=table.get("title", name), kind=kind, modules=modules,
                                   executors=list(executors), runner_args=list(table.get("runner_args", [])))
    return scenarios


def check_pair(stand, scenario):
    """The scenario's module set exists in the stand description."""
    if scenario.modules and scenario.modules not in stand.modules:
        raise DescriptionError(f"stand {stand.name!r} has no module set {scenario.modules!r} "
                               f"needed by scenario {scenario.name!r}")
