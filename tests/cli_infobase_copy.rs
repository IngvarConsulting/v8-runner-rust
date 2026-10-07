//! `infobase create --from <база>` (#330): база рабочей копии — копия другой объявленной базы.
//!
//! Поддельный Конфигуратор снимает образ (`/DumpIB`) — копирует в него файл файловой базы из строки соединения, а
//! при файле `busy` рядом с собой отказывает, как занятая база; поддельный `ibcmd` создаёт
//! базу из образа (`infobase restore --create-database`) и отвечает поколением. Оба пишут
//! вызовы в журнал.
#![cfg(unix)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::Value;
use support::{temp_workspace, v8_runner_command, write_shell_script};

const TOKEN: &str = "3333333333333333333333333333333333333333";

/// Поддельные утилиты платформы в `bin` под `root`.
fn write_platform(root: &Path) -> PathBuf {
    let bin = root.join("bin");
    fs::create_dir_all(&bin).expect("bin");
    let calls = root.join("calls.log");
    write_shell_script(
        &bin.join("1cv8"),
        &format!(
            r#"printf '1cv8 %s\n' "$*" >> '{calls}'
out=''
base=''
dump=''
previous=''
for arg in "$@"; do
  case "$previous" in
    /Out) out="$arg" ;;
    /IBConnectionString) base="${{arg#File=}}"; base="${{base%;}}"; base="${{base#[\"\']}}"; base="${{base%[\"\']}}" ;;
    /DumpIB) dump="$arg" ;;
  esac
  previous="$arg"
done
if [ -n "$dump" ]; then
  if [ -f '{busy}' ]; then
    printf 'the infobase is held exclusively\n' >&2
    exit 1
  fi
  if [ -f "$base/1Cv8.1CD" ]; then cat "$base/1Cv8.1CD" > "$dump"; else printf 'server image\n' > "$dump"; fi
  exit 0
fi
case "$*" in
  *'/GetConfigGenerationID'*) printf '{TOKEN}\r\n' > "$out"; exit 0 ;;
esac
if [ -n "$out" ]; then : > "$out"; fi
exit 0"#,
            calls = calls.display(),
            busy = root.join("busy").display(),
        ),
    );
    write_shell_script(
        &bin.join("ibcmd"),
        &format!(
            r#"printf 'ibcmd %s\n' "$*" >> '{calls}'
command=''
path=''
image=''
for arg in "$@"; do image="$arg"; done
previous=''
for arg in "$@"; do
  case "$arg" in
    create|restore) if [ -z "$command" ]; then command="$arg"; fi ;;
    generation-id) command=generation ;;
  esac
  if [ "$previous" = '--db-path' ]; then path="$arg"; fi
  previous="$arg"
done
case "$command" in
  create) mkdir -p "$path" && printf 'database of %s\n' "$path" > "$path/1Cv8.1CD" ;;
  restore) mkdir -p "$path" && cat "$image" > "$path/1Cv8.1CD" ;;
  generation) printf '{TOKEN}\n' ;;
esac
exit 0"#,
            calls = calls.display(),
        ),
    );
    bin.join("1cv8")
}

/// Рабочая копия: проект с основной конфигурацией и местный слой `local`.
fn write_copy(dir: &Path, platform: &Path, local: &str) -> PathBuf {
    let sources = dir.join("src");
    fs::create_dir_all(&sources).expect("sources");
    fs::write(sources.join("Configuration.xml"), "<Configuration/>\n").expect("source");
    fs::write(sources.join("Module.bsl"), "Procedure A()\nEndProcedure\n").expect("module");
    let config = dir.join("v8project.yaml");
    fs::write(
        &config,
        format!(
            "workPath: work\nformat: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\ntools:\n  platform:\n    path: '{}'\n",
            platform.display()
        ),
    )
    .expect("config");
    fs::write(dir.join("v8project.local.yaml"), local).expect("local layer");
    config
}

fn run(config: &Path, args: &[&str]) -> Output {
    v8_runner_command()
        .arg("--config")
        .arg(config)
        .arg("--json-message")
        .args(args)
        .output()
        .expect("run CLI")
}

/// `init --infobase File=build/ib` в каталоге копии: `origin` — своя база, прежняя секция —
/// `upstream`.
fn init_own_base(config: &Path) {
    let output = v8_runner_command()
        .current_dir(config.parent().expect("project dir"))
        .args(["--json-message", "init", "--infobase", "File=build/ib"])
        .output()
        .expect("run init");
    succeeded(&output);
}

fn envelope(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "json: {error}; stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn succeeded(output: &Output) -> Value {
    let payload = envelope(output);
    assert!(output.status.success(), "{payload}");
    payload
}

fn calls(root: &Path) -> String {
    fs::read_to_string(root.join("calls.log")).unwrap_or_default()
}

fn canonical(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Метка владельца рядом с каталогом файловой базы.
fn owner_marker(base: &Path) -> PathBuf {
    base.with_file_name(format!(
        ".{}.v8-runner.owners.json",
        base.file_name().expect("name").to_string_lossy()
    ))
}

/// Проекты, записанные в метке базы.
fn owners(base: &Path) -> Vec<PathBuf> {
    let marker: Value =
        serde_json::from_slice(&fs::read(owner_marker(base)).expect("marker")).expect("json");
    marker["owners"]
        .as_array()
        .expect("owners")
        .iter()
        .map(|owner| canonical(Path::new(owner["project"].as_str().expect("project"))))
        .collect()
}

fn step_modes(pushed: &Value) -> Vec<(String, String)> {
    pushed["data"]["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .map(|step| {
            (
                step["source_set"].as_str().unwrap_or_default().to_owned(),
                step["mode"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

/// Две рабочие копии одного проекта: соседняя держит свою базу, вторая — ворктри с её
/// местным слоем, где `origin` указывает на базу соседа.
struct Stand {
    dir: tempfile::TempDir,
    platform: PathBuf,
}

impl Stand {
    fn new() -> Self {
        let dir = temp_workspace();
        let platform = write_platform(dir.path());
        Self { dir, platform }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    /// Соседняя копия `erp` с базой `erp/build/ib`, которую она создала и держит.
    fn neighbour(&self) -> (PathBuf, PathBuf) {
        let erp = self.root().join("erp");
        let base = erp.join("build").join("ib");
        let config = write_copy(
            &erp,
            &self.platform,
            &format!(
                "infobases:\n  origin:\n    connection: 'File={}'\n",
                base.display()
            ),
        );
        succeeded(&run(&config, &["infobase", "create"]));
        (config, base)
    }

    /// Ворктри `wt` с местным слоем соседа: `origin` — база соседа.
    fn worktree(&self, neighbour_base: &Path) -> PathBuf {
        write_copy(
            &self.root().join("wt"),
            &self.platform,
            &format!(
                "infobases:\n  origin:\n    connection: 'File={}'\n",
                neighbour_base.display()
            ),
        )
    }
}

/// Сценарий «Отладка на копии базы» целиком: `init --infobase File=build/ib`, затем
/// `infobase create --from upstream` и полная первая отправка. База соседа не тронута и
/// остаётся за ним; новая база — за этой копией.
#[test]
fn debugging_on_a_copy_of_the_base_leaves_the_neighbour_untouched() {
    let stand = Stand::new();
    let (neighbour, neighbour_base) = stand.neighbour();
    let neighbour_data = fs::read(neighbour_base.join("1Cv8.1CD")).expect("neighbour base");
    let neighbour_marker = fs::read(owner_marker(&neighbour_base)).expect("neighbour marker");
    let worktree = stand.worktree(&neighbour_base);
    let wt = worktree.parent().expect("worktree").to_path_buf();

    init_own_base(&worktree);
    let copied = succeeded(&run(
        &worktree,
        &["infobase", "create", "--from", "upstream"],
    ));

    let base = wt.join("build").join("ib");
    let snapshot = canonical(&wt.join("work").join("copies").join("upstream.dt"));
    assert_eq!(copied["data"]["source"]["infobase"], "upstream", "{copied}");
    assert_eq!(
        canonical(Path::new(
            copied["data"]["source"]["snapshot"]
                .as_str()
                .expect("snapshot")
        )),
        snapshot,
        "{copied}"
    );
    assert_eq!(
        fs::read(base.join("1Cv8.1CD")).expect("copied base"),
        neighbour_data,
        "the new infobase holds the data of the source"
    );
    let log = calls(stand.root());
    let dump = log
        .lines()
        .find(|line| line.contains("/DumpIB"))
        .expect("snapshot call");
    assert!(
        dump.contains(&neighbour_base.display().to_string()),
        "{dump}"
    );
    let restore = log
        .lines()
        .find(|line| line.contains(" restore "))
        .expect("restore call");
    assert!(restore.contains("--create-database"), "{restore}");

    assert_eq!(
        fs::read(neighbour_base.join("1Cv8.1CD")).expect("neighbour base"),
        neighbour_data,
        "the base of the neighbour is untouched"
    );
    assert_eq!(
        fs::read(owner_marker(&neighbour_base)).expect("neighbour marker"),
        neighbour_marker,
        "the copy is not written into the marker of the source"
    );
    assert_eq!(owners(&base), vec![canonical(&wt)]);

    let pushed = succeeded(&run(&worktree, &["push"]));
    assert_eq!(
        step_modes(&pushed),
        vec![("main".to_owned(), "full".to_owned())],
        "{pushed}"
    );
    let again = succeeded(&run(&worktree, &["push"]));
    assert_eq!(
        step_modes(&again),
        vec![("main".to_owned(), "skipped".to_owned())],
        "the first push remembered the sources: {again}"
    );

    succeeded(&run(&neighbour, &["push", "--force"]));
    assert_eq!(
        owners(&neighbour_base),
        vec![canonical(
            &neighbour_base.parent().unwrap().parent().unwrap()
        )]
    );
}

/// Память копии — только признак копии и поколение новой базы: прежняя память под именем
/// базы стирается, первая отправка идёт полной и отказом первого знакомства не
/// останавливается.
#[test]
fn a_copied_base_starts_with_a_full_push() {
    let stand = Stand::new();
    let (_neighbour, neighbour_base) = stand.neighbour();
    let worktree = stand.worktree(&neighbour_base);
    let wt = worktree.parent().expect("worktree").to_path_buf();
    init_own_base(&worktree);
    let memory = wt.join("work").join("infobases").join("origin");
    fs::create_dir_all(memory.join("dump-info").join("main")).expect("stale memory");
    fs::write(memory.join("generation.json"), "{}").expect("stale ledger");

    succeeded(&run(
        &worktree,
        &["infobase", "create", "--from", "upstream"],
    ));

    let mark: Value =
        serde_json::from_slice(&fs::read(memory.join("copied-from.json")).expect("the copy mark"))
            .expect("json");
    assert_eq!(mark["source"], "upstream", "{mark}");
    assert_eq!(mark["generation"]["tool"], "ibcmd", "{mark}");
    assert_eq!(mark["generation"]["token"], TOKEN, "{mark}");
    for stale in ["hashes", "dump-info", "generation.json"] {
        assert!(!memory.join(stale).exists(), "{stale} is erased");
    }
    let status = succeeded(&run(&worktree, &["status"]));
    assert_eq!(
        status["data"]["infobases"][0]["source_sets"][0]["memory"], "remembered",
        "{status}"
    );

    let pushed = succeeded(&run(&worktree, &["push"]));

    assert_eq!(
        step_modes(&pushed),
        vec![("main".to_owned(), "full".to_owned())],
        "{pushed}"
    );
    assert!(
        !memory.join("copied-from.json").exists(),
        "the first push removes the copy mark"
    );
}

/// Неудавшийся снимок файловой базы называет, как освободить источник, и копию, которая
/// держит его; новой базы нет, образ убран, метка источника не тронута.
#[test]
fn a_failed_snapshot_of_a_file_base_names_the_recipe() {
    let stand = Stand::new();
    let (neighbour, neighbour_base) = stand.neighbour();
    let neighbour_marker = fs::read(owner_marker(&neighbour_base)).expect("neighbour marker");
    let worktree = stand.worktree(&neighbour_base);
    let wt = worktree.parent().expect("worktree").to_path_buf();
    init_own_base(&worktree);
    fs::write(stand.root().join("busy"), "").expect("busy");

    let output = run(&worktree, &["infobase", "create", "--from", "upstream"]);

    assert!(!output.status.success());
    let payload = envelope(&output);
    let message = payload["error"]["message"].as_str().expect("message");
    assert!(
        message.contains("close the Designer and the clients of the working copy that holds it")
            && message.contains(&canonical(neighbour.parent().unwrap()).display().to_string())
            && message.contains("the runner ends no sessions itself"),
        "{message}"
    );
    assert!(!wt.join("build").join("ib").join("1Cv8.1CD").exists());
    assert!(!wt.join("work").join("copies").join("upstream.dt").exists());
    assert!(!calls(stand.root()).contains(" restore "));
    assert_eq!(
        fs::read(owner_marker(&neighbour_base)).expect("neighbour marker"),
        neighbour_marker
    );
}

/// Неудавшийся снимок базы в кластере называет окно обслуживания: `sessions deny` и
/// `sessions terminate`, после снимка — `sessions allow`.
#[test]
fn a_failed_snapshot_of_a_cluster_base_names_the_maintenance_window() {
    let stand = Stand::new();
    let wt = stand.root().join("wt");
    let worktree = write_copy(
        &wt,
        &stand.platform,
        "infobases:\n  origin:\n    connection: 'File=build/ib'\n  upstream:\n    connection: 'Srvr=cluster:1541;Ref=erp'\n    user: Admin\n    password: s3cret\n",
    );
    fs::write(stand.root().join("busy"), "").expect("busy");

    let output = run(&worktree, &["infobase", "create", "--from", "upstream"]);

    assert!(!output.status.success());
    let payload = envelope(&output);
    let message = payload["error"]["message"].as_str().expect("message");
    for part in [
        "`sessions deny`",
        "`sessions terminate`",
        "`sessions allow`",
        "the runner ends no sessions itself",
    ] {
        assert!(message.contains(part), "{part}: {message}");
    }
    assert!(!message.contains("s3cret"), "{message}");
    assert!(!wt.join("build").join("ib").exists());
}

/// Источник на автономном сервере — отказ до снимка с рецептом снимка на машине сервера; и
/// под превью, и в прогоне платформа не запускается.
#[test]
fn a_standalone_source_is_refused_before_the_snapshot() {
    let stand = Stand::new();
    let worktree = write_copy(
        &stand.root().join("wt"),
        &stand.platform,
        "infobases:\n  origin:\n    connection: 'File=build/ib'\n  upstream:\n    user: gate\n    standalone:\n      gate: 127.0.0.1:1543\n      exchange: sftp\n",
    );

    for extra in [&["--dry-run"][..], &[][..]] {
        let mut args = vec!["infobase", "create", "--from", "upstream"];
        args.extend_from_slice(extra);
        let output = run(&worktree, &args);

        assert!(!output.status.success(), "{extra:?}");
        let payload = envelope(&output);
        assert_eq!(payload["error"]["kind"], "capability", "{payload}");
        assert_eq!(payload["error"]["code"], "target", "{payload}");
        let message = payload["error"]["message"].as_str().expect("message");
        assert!(
            message.contains("on the server machine")
                && message.contains("ibcmd infobase dump")
                && message.contains("infobase restore --input <file>.dt --create"),
            "{message}"
        );
    }
    assert!(calls(stand.root()).is_empty(), "{}", calls(stand.root()));
}

/// Превью называет источник, снимок и создание, ничего не снимая и не создавая; источник,
/// которого нет в местном слое, и сама создаваемая база — отказ до платформы.
#[test]
fn a_preview_names_the_snapshot_and_a_wrong_source_is_refused() {
    let stand = Stand::new();
    let worktree = write_copy(
        &stand.root().join("wt"),
        &stand.platform,
        "infobases:\n  origin:\n    connection: 'File=build/ib'\n  upstream:\n    connection: 'File=/srv/erp-ib'\n",
    );

    let preview = succeeded(&run(
        &worktree,
        &["infobase", "create", "--from", "upstream", "--dry-run"],
    ));
    assert_eq!(
        preview["data"]["steps"][0]["status"], "planned",
        "{preview}"
    );
    assert_eq!(
        preview["data"]["source"]["infobase"], "upstream",
        "{preview}"
    );
    let message = preview["data"]["steps"][0]["message"]
        .as_str()
        .expect("message");
    assert!(
        message.contains("/DumpIB") && message.contains("restore --create-database"),
        "{message}"
    );

    for from in ["missing", "origin"] {
        let output = run(&worktree, &["infobase", "create", "--from", from]);
        assert!(!output.status.success(), "{from}");
        let payload = envelope(&output);
        assert_eq!(payload["error"]["kind"], "validation", "{payload}");
    }
    assert!(calls(stand.root()).is_empty(), "{}", calls(stand.root()));
}

/// База в кластере из образа: Конфигуратор создаёт её `CREATEINFOBASE`, затем загружает образ
/// `/RestoreIB`; память — признак копии, и первая отправка полная.
#[test]
fn a_cluster_copy_is_created_by_the_designer_and_loaded_from_the_image() {
    let stand = Stand::new();
    let (_neighbour, neighbour_base) = stand.neighbour();
    let wt = stand.root().join("wt");
    let worktree = write_copy(
        &wt,
        &stand.platform,
        &format!(
            "infobases:\n  origin:\n    connection: 'Srvr=cluster:1541;Ref=wt'\n    dbms:\n      kind: PostgreSQL\n      server: db\n      name: wt_db\n      user: postgres\n      password: pg-s3cret\n      locale: ru\n  upstream:\n    connection: 'File={}'\n",
            neighbour_base.display()
        ),
    );

    let copied = succeeded(&run(
        &worktree,
        &["infobase", "create", "--from", "upstream"],
    ));

    assert_eq!(copied["data"]["source"]["infobase"], "upstream", "{copied}");
    let log = calls(stand.root());
    let order: Vec<&str> = log
        .lines()
        .filter_map(|line| {
            ["/DumpIB", "CREATEINFOBASE", "/RestoreIB"]
                .into_iter()
                .find(|call| line.contains(call))
        })
        .collect();
    assert_eq!(order, ["/DumpIB", "CREATEINFOBASE", "/RestoreIB"], "{log}");
    let restore = log
        .lines()
        .find(|line| line.contains("/RestoreIB"))
        .expect("restore");
    assert!(restore.contains("upstream.dt"), "{restore}");
    assert!(!copied.to_string().contains("pg-s3cret"), "{copied}");
    let memory = wt.join("work").join("infobases").join("origin");
    assert!(memory.join("copied-from.json").exists());
}
