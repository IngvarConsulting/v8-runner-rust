//! Rules of git ignore files that can hide a file the platform writes into a source set (#575).
//!
//! Changes of a set inside a git working tree come from git, and git does not report a file it
//! ignores. A rule that can hide such a file refuses the command
//! (`INV.USE-CASES.A-SET-HIDDEN-BY-A-GIT-IGNORE-RULE-IS-REFUSED`). The rules are read and
//! matched here, the way git matches them: the last matching rule decides, a negation lets a
//! path through, a directory that is ignored hides everything below it, rules of a deeper
//! `.gitignore` are stronger than those of a shallower one, `.git/info/exclude` and
//! `core.excludesFile` are the weakest.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use regex::{Regex, RegexSet};
use serde::{Deserialize, Serialize};

use crate::change_detection::hash_storage::{StorageSnapshot, StorageSummary};
use crate::domain::git_worktree::GitIgnoreSource;

/// File extensions a Designer dump writes (#575): the paths a rule is tried on while the set
/// has no memory of the base yet.
const DUMP_EXTENSIONS: &[&str] = &[
    "xml", "bsl", "bin", "txt", "html", "htm", "png", "gif", "jpg", "jpeg", "bmp", "ico", "svg",
    "tif", "tiff", "wmf", "emf",
];

/// How many hidden paths a refusal names.
const NAMED_PATHS: usize = 5;

/// One rule line of an ignore file.
struct Rule {
    /// The line as written, without the line end.
    text: String,
    file: PathBuf,
    /// Line number from 1.
    line: usize,
    negated: bool,
}

/// Rules matched the same way — from one directory, against the path below it or against
/// the name, for any entry or for directories only — in one [`RegexSet`], so a path is tried
/// against all of them in one pass.
struct Group {
    base: String,
    anchored: bool,
    dir_only: bool,
    set: RegexSet,
    /// Index of each pattern of `set` in [`IgnoreRules::rules`].
    rules: Vec<usize>,
}

/// What rules of one [`Group`] share: their directory, whether they hold a slash, whether
/// they match directories only.
type GroupKey = (String, bool, bool);

/// The rules of every ignore file in force, weakest first.
pub(crate) struct IgnoreRules {
    rules: Vec<Rule>,
    groups: Vec<Group>,
    /// Rules whose pattern could not be built: they are taken to match every path, so such a
    /// rule is named in the refusal instead of silently dropped.
    broken: Vec<usize>,
    ignore_case: bool,
}

/// A rule that hides files of a set, with the first of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiddenBy {
    /// The rule as written.
    pub rule: String,
    /// The ignore file that holds it.
    pub file: PathBuf,
    /// Its line in that file, from 1.
    pub line: usize,
    /// The first hidden paths below the set root, `/`-separated; the root itself is `.`.
    pub paths: Vec<String>,
}

impl IgnoreRules {
    /// Reads every source; a file that is absent or unreadable holds no rules, as for git.
    pub(crate) fn read(sources: &[GitIgnoreSource], ignore_case: bool) -> Self {
        let mut rules = Vec::new();
        let mut grouped: Vec<(GroupKey, Vec<(String, usize)>)> = Vec::new();
        let mut broken = Vec::new();
        for source in sources {
            let Ok(bytes) = std::fs::read(&source.file) else {
                continue;
            };
            let text = String::from_utf8_lossy(&bytes);
            for (index, line) in text.split('\n').enumerate() {
                let Some(parsed) = parse(line, ignore_case) else {
                    continue;
                };
                let at = rules.len();
                let compiled = Regex::new(&parsed.regex).is_ok();
                if compiled {
                    let key = (source.base.clone(), parsed.anchored, parsed.dir_only);
                    match grouped.iter_mut().find(|(group, _)| *group == key) {
                        Some((_, patterns)) => patterns.push((parsed.regex.clone(), at)),
                        None => grouped.push((key, vec![(parsed.regex.clone(), at)])),
                    }
                } else {
                    broken.push(at);
                }
                rules.push(Rule {
                    text: parsed.text,
                    file: source.file.clone(),
                    line: index + 1,
                    negated: parsed.negated,
                });
            }
        }
        let mut groups = Vec::new();
        for ((base, anchored, dir_only), patterns) in grouped {
            match RegexSet::new(patterns.iter().map(|(regex, _)| regex)) {
                Ok(set) => groups.push(Group {
                    base,
                    anchored,
                    dir_only,
                    set,
                    rules: patterns.into_iter().map(|(_, at)| at).collect(),
                }),
                Err(_) => broken.extend(patterns.into_iter().map(|(_, at)| at)),
            }
        }
        Self {
            rules,
            groups,
            broken,
            ignore_case,
        }
    }

    /// The rule that hides the first of `candidates` (paths below the set root at `root`,
    /// from the root of the working tree) or the root itself, with up to [`NAMED_PATHS`] of
    /// the paths it hides. `None`: no rule hides any of them.
    pub(crate) fn hiding<'c>(
        &self,
        root: &str,
        candidates: impl IntoIterator<Item = &'c str>,
    ) -> Option<HiddenBy> {
        let mut directories = HashMap::new();
        if !root.is_empty() {
            if let Some(rule) = self.verdict(root, true, &mut directories) {
                return Some(self.hit(rule, vec![".".to_owned()]));
            }
        }
        let mut found: Option<(usize, Vec<String>)> = None;
        let mut path = String::new();
        for candidate in candidates {
            path.clear();
            if !root.is_empty() {
                path.push_str(root);
                path.push('/');
            }
            path.push_str(candidate);
            let Some(rule) = self.verdict(&path, false, &mut directories) else {
                continue;
            };
            match &mut found {
                None => found = Some((rule, vec![candidate.to_owned()])),
                Some((first, paths)) if *first == rule => {
                    paths.push(candidate.to_owned());
                    if paths.len() >= NAMED_PATHS {
                        break;
                    }
                }
                Some(_) => {}
            }
        }
        found.map(|(rule, paths)| self.hit(rule, paths))
    }

    fn hit(&self, rule: usize, paths: Vec<String>) -> HiddenBy {
        let rule = &self.rules[rule];
        HiddenBy {
            rule: rule.text.clone(),
            file: rule.file.clone(),
            line: rule.line,
            paths,
        }
    }

    /// The rule that ignores `path`: a parent directory that is ignored first, then the last
    /// rule that matches the path itself, unless that one is a negation. Every directory is
    /// decided once.
    fn verdict(
        &self,
        path: &str,
        is_dir: bool,
        directories: &mut HashMap<String, Option<usize>>,
    ) -> Option<usize> {
        if let Some((parent, _)) = path.rsplit_once('/') {
            let parent_verdict = match directories.get(parent) {
                Some(verdict) => *verdict,
                None => {
                    let verdict = self.verdict(parent, true, directories);
                    directories.insert(parent.to_owned(), verdict);
                    verdict
                }
            };
            if parent_verdict.is_some() {
                return parent_verdict;
            }
        }
        let mut last = self.broken.iter().copied().max();
        for group in &self.groups {
            if group.dir_only && !is_dir {
                continue;
            }
            let below = if group.base.is_empty() {
                path
            } else {
                match strip_dir_prefix(path, &group.base, self.ignore_case) {
                    Some(below) => below,
                    None => continue,
                }
            };
            let subject = if group.anchored {
                below
            } else {
                below.rsplit('/').next().unwrap_or(below)
            };
            if let Some(matched) = group.set.matches(subject).iter().next_back() {
                let rule = group.rules[matched];
                last = Some(last.map_or(rule, |current| current.max(rule)));
            }
        }
        let last = last?;
        (!self.rules[last].negated).then_some(last)
    }
}

/// One rule line, parsed.
struct Parsed {
    text: String,
    negated: bool,
    dir_only: bool,
    anchored: bool,
    regex: String,
}

fn parse(raw: &str, ignore_case: bool) -> Option<Parsed> {
    let text = raw.strip_suffix('\r').unwrap_or(raw);
    if text.is_empty() || text.starts_with('#') {
        return None;
    }
    let mut pattern = trim_unescaped_trailing_spaces(text);
    let negated = pattern.starts_with('!');
    if negated {
        pattern = &pattern[1..];
    }
    let dir_only = pattern.ends_with('/') && !pattern.ends_with("\\/");
    let pattern = pattern
        .strip_suffix('/')
        .filter(|_| dir_only)
        .unwrap_or(pattern);
    if pattern.is_empty() {
        return None;
    }
    let anchored = pattern.contains('/');
    let pattern = pattern.strip_prefix('/').unwrap_or(pattern);
    Some(Parsed {
        text: text.to_owned(),
        negated,
        dir_only,
        anchored,
        regex: format!(
            "{}^{}$",
            if ignore_case { "(?i)" } else { "" },
            glob_to_regex(pattern)
        ),
    })
}

/// `path` below the directory `dir`, both from the root of the working tree.
fn strip_dir_prefix<'a>(path: &'a str, dir: &str, ignore_case: bool) -> Option<&'a str> {
    let head = path.get(..dir.len())?;
    let same = if ignore_case {
        head.eq_ignore_ascii_case(dir)
    } else {
        head == dir
    };
    (same && path.as_bytes().get(dir.len()) == Some(&b'/')).then(|| &path[dir.len() + 1..])
}

/// Trailing spaces end a pattern unless a backslash escapes them.
fn trim_unescaped_trailing_spaces(text: &str) -> &str {
    let mut end = text.len();
    while end > 0 && text.as_bytes()[end - 1] == b' ' {
        if end >= 2 && text.as_bytes()[end - 2] == b'\\' {
            break;
        }
        end -= 1;
    }
    &text[..end]
}

/// The index of the `]` that closes the class opened at `open`: a `]` right after `[`, `[!` or
/// `[^` is a member, and a POSIX class `[:name:]` inside is skipped whole. `None`: the bracket
/// is a literal.
fn class_end(chars: &[char], open: usize) -> Option<usize> {
    let mut i = open + 1;
    if matches!(chars.get(i), Some('!' | '^')) {
        i += 1;
    }
    if chars.get(i) == Some(&']') {
        i += 1;
    }
    while i < chars.len() {
        match chars[i] {
            ']' => return Some(i),
            '[' if chars.get(i + 1) == Some(&':') => {
                let close = (i + 2..chars.len().saturating_sub(1))
                    .find(|&at| chars[at] == ':' && chars[at + 1] == ']')?;
                i = close + 2;
            }
            _ => i += 1,
        }
    }
    None
}

/// The inside of a bracket class: POSIX classes such as `[:alpha:]` pass as they are, other
/// characters the regular expression syntax treats specially are escaped.
fn class_to_regex(class: &str) -> String {
    let mut out = String::new();
    let mut rest = class;
    while let Some(c) = rest.chars().next() {
        if let Some(inner) = rest.strip_prefix("[:") {
            if let Some(end) = inner.find(":]") {
                out.push_str(&rest[..end + 4]);
                rest = &rest[end + 4..];
                continue;
            }
        }
        if matches!(c, '\\' | '[' | ']' | '&' | '~') {
            out.push('\\');
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// A git wildcard pattern as a regular expression over a `/`-separated path: `*` and `?` stay
/// inside one component, `**` between slashes crosses any number of them.
fn glob_to_regex(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut regex = String::new();
    let mut i = 0;
    while i < chars.len() {
        let at_start = i == 0 || chars[i - 1] == '/';
        match chars[i] {
            '*' if chars.get(i + 1) == Some(&'*') && at_start => {
                let end = i + 2;
                if end == chars.len() {
                    regex.push_str(".*");
                    i = end;
                } else if chars[end] == '/' {
                    regex.push_str("(?:.*/)?");
                    i = end + 1;
                } else {
                    regex.push_str("[^/]*");
                    i = end;
                }
            }
            '*' => {
                regex.push_str("[^/]*");
                i += 1;
                while chars.get(i) == Some(&'*') {
                    i += 1;
                }
            }
            '?' => {
                regex.push_str("[^/]");
                i += 1;
            }
            '[' => match class_end(&chars, i) {
                Some(close) => {
                    let class: String = chars[i + 1..close].iter().collect();
                    regex.push('[');
                    let class = match class.strip_prefix(['!', '^']) {
                        Some(rest) => {
                            // A negated class never matches the separator.
                            regex.push_str("^/");
                            rest.to_owned()
                        }
                        None => class,
                    };
                    regex.push_str(&class_to_regex(&class));
                    regex.push(']');
                    i = close + 1;
                }
                None => {
                    regex.push_str("\\[");
                    i += 1;
                }
            },
            '\\' if i + 1 < chars.len() => {
                regex.push_str(&regex::escape(&chars[i + 1].to_string()));
                i += 2;
            }
            c => {
                regex.push_str(&regex::escape(&c.to_string()));
                i += 1;
            }
        }
    }
    regex
}

/// Version of what a [`CheckInput`] describes. Raise it with every change of the matching
/// ([`IgnoreRules`]), of [`dump_samples`] or of the paths a check is tried on: builds of one
/// version share their records, and a record of the old answer must not be taken.
const CHECK_FORMAT: u32 = 1;

/// A rules file younger than this may still change within the same time stamp and size — FAT
/// keeps the time in steps of two seconds — so a check that read it is not recorded (#645).
const SETTLED_NS: u64 = 3_000_000_000;

/// Everything the answer of the ignore check of one set depends on (#645): the build of the
/// runner, the set root and the runner directories below it, how names compare, every rules
/// file with its time and size, and the memory the rules were tried on. The same input gives
/// the same answer, so a check that hid nothing is not repeated while its input stays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CheckInput {
    format: u32,
    runner: String,
    root: String,
    ignore_case: bool,
    runner_dirs: Vec<String>,
    sources: Vec<SourceStamp>,
    memory: MemoryStamp,
}

/// A rules file as the check saw it; `None` — absent or unreadable, which holds no rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SourceStamp {
    file: PathBuf,
    base: String,
    modified_ns: Option<u64>,
    len: Option<u64>,
}

/// The paths the rules were tried on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum MemoryStamp {
    /// No remembered file: the samples of a dump ([`dump_samples`]).
    Samples,
    /// The files of the memory of this generation; a commit always changes the generation.
    Remembered {
        identity: Option<String>,
        generation: u64,
        files: u64,
    },
}

impl MemoryStamp {
    /// The paths of `snapshot` the rules are tried on.
    pub(crate) fn of_snapshot(snapshot: &StorageSnapshot) -> Self {
        if snapshot.entries.is_empty() {
            Self::Samples
        } else {
            Self::Remembered {
                identity: snapshot.identity.clone(),
                generation: snapshot.generation,
                files: snapshot.entries.len() as u64,
            }
        }
    }

    /// The same answer as [`Self::of_snapshot`] for the snapshot `summary` describes.
    pub(crate) fn of_summary(summary: Option<&StorageSummary>) -> Self {
        match summary {
            Some(summary) if summary.files > 0 => Self::Remembered {
                identity: summary.identity.clone(),
                generation: summary.generation,
                files: summary.files,
            },
            _ => Self::Samples,
        }
    }
}

impl CheckInput {
    /// The input of the check of the set at `root`, and whether every rules file is old enough
    /// for the answer to be recorded.
    pub(crate) fn of(
        sources: &[GitIgnoreSource],
        root: &str,
        runner_dirs: &[String],
        ignore_case: bool,
        memory: MemoryStamp,
    ) -> (Self, bool) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| {
                u64::try_from(since.as_nanos()).unwrap_or(u64::MAX)
            });
        let mut settled = true;
        let sources = sources
            .iter()
            .map(|source| {
                let meta = std::fs::metadata(&source.file).ok();
                let modified_ns = meta
                    .as_ref()
                    .and_then(|meta| meta.modified().ok())
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .and_then(|since| u64::try_from(since.as_nanos()).ok());
                if modified_ns.is_some_and(|at| at.saturating_add(SETTLED_NS) > now) {
                    settled = false;
                }
                SourceStamp {
                    file: source.file.clone(),
                    base: source.base.clone(),
                    modified_ns,
                    len: meta.map(|meta| meta.len()),
                }
            })
            .collect();
        let input = Self {
            format: CHECK_FORMAT,
            runner: env!("CARGO_PKG_VERSION").to_owned(),
            root: root.to_owned(),
            ignore_case,
            runner_dirs: runner_dirs.to_vec(),
            sources,
            memory,
        };
        (input, settled)
    }

    /// The same input tried on another memory.
    pub(crate) fn with_memory(mut self, memory: MemoryStamp) -> Self {
        self.memory = memory;
        self
    }
}

/// The record of a check that hid nothing, one file per set under `workPath` (#645). A record
/// that cannot be read or does not match is no record; one that cannot be written is skipped.
pub(crate) struct CheckRecord {
    work_path: PathBuf,
    path: PathBuf,
}

impl CheckRecord {
    /// The record of the set whose memory — or, without memory, whose directory — is `place`,
    /// under `work_path`.
    pub(crate) fn at(work_path: &Path, place: &Path) -> Self {
        use sha2::Digest;
        let digest = sha2::Sha256::digest(place.to_string_lossy().as_bytes());
        let name: String = digest[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Self {
            work_path: work_path.to_path_buf(),
            path: work_path.join("ignore-check").join(format!("{name}.json")),
        }
    }

    fn read(&self) -> Option<CheckInput> {
        std::fs::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    /// Whether the check already ran on `input` and hid nothing.
    pub(crate) fn holds(&self, input: &CheckInput) -> bool {
        self.read().is_some_and(|recorded| recorded == *input)
    }

    /// Carries the record over a commit of the memory (#645). The memory of `generation` the
    /// record was taken on became `to`; its files are those it held and those the commit wrote,
    /// `written`. The rules files still as recorded, a check of `written` alone gives the answer
    /// of the whole memory: the record then describes `to`. Anything else leaves the record as
    /// it is, and the next command checks the whole memory.
    pub(crate) fn carry<'p>(
        &self,
        generation: u64,
        to: MemoryStamp,
        written: impl IntoIterator<Item = &'p str>,
    ) {
        let Some(recorded) = self.read() else {
            return;
        };
        let carried = match (&recorded.memory, &to) {
            (
                MemoryStamp::Remembered {
                    identity: before,
                    generation: was,
                    ..
                },
                MemoryStamp::Remembered { identity, .. },
            ) => *was == generation && before == identity,
            _ => false,
        };
        if !carried {
            return;
        }
        let sources: Vec<GitIgnoreSource> = recorded
            .sources
            .iter()
            .map(|source| GitIgnoreSource {
                file: source.file.clone(),
                base: source.base.clone(),
            })
            .collect();
        let (now, settled) = CheckInput::of(
            &sources,
            &recorded.root,
            &recorded.runner_dirs,
            recorded.ignore_case,
            to.clone(),
        );
        if !settled || now != recorded.clone().with_memory(to) {
            return;
        }
        let rules = IgnoreRules::read(&sources, recorded.ignore_case);
        let mut written: Vec<&str> = written
            .into_iter()
            .filter(|path| {
                !crate::change_detection::git_tree::in_runner_dir(
                    path,
                    &recorded.runner_dirs,
                    recorded.ignore_case,
                )
            })
            .collect();
        written.sort_unstable();
        if rules.hiding(&recorded.root, written).is_none() {
            self.keep(&now);
            tracing::debug!(
                record = %self.path.display(),
                "the recorded ignore check was carried over to the new memory"
            );
        }
    }

    /// Records that the check of `input` hid nothing. The `workPath` itself is not created: a
    /// command that has none yet records nothing.
    pub(crate) fn keep(&self, input: &CheckInput) {
        if !self.work_path.is_dir() {
            return;
        }
        let written = serde_json::to_vec(input)
            .map_err(std::io::Error::other)
            .and_then(|bytes| {
                if let Some(dir) = self.path.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                crate::support::fs::write_file_atomically(&self.path, |file| {
                    use std::io::Write;
                    file.write_all(&bytes)
                })
            });
        if let Err(error) = written {
            tracing::debug!(record = %self.path.display(), %error, "the ignore check was not recorded");
        }
    }
}

/// The paths a dump of a configuration writes, as samples below a set root: every folder of
/// the dump layout ([`crate::change_detection::partial_load::dump_layout_folders`]) with an
/// element, its `Ext` folder and a file of every extension a dump writes. A set without
/// memory of the base is checked against them.
pub(crate) fn dump_samples() -> Vec<String> {
    let (top, nested) = crate::change_detection::partial_load::dump_layout_folders();
    let mut samples = Vec::new();
    for extension in DUMP_EXTENSIONS {
        samples.push(format!("Configuration.{extension}"));
        samples.push(format!("Ext/Sample.{extension}"));
        for folder in &top {
            samples.push(format!("{folder}/Sample.{extension}"));
            samples.push(format!("{folder}/Sample/Ext/Sample.{extension}"));
            for inner in &nested {
                samples.push(format!("{folder}/Sample/{inner}/Sample.{extension}"));
                samples.push(format!(
                    "{folder}/Sample/{inner}/Sample/Ext/Sample.{extension}"
                ));
            }
        }
    }
    samples
}

#[cfg(test)]
mod tests {
    use super::{dump_samples, CheckInput, CheckRecord, IgnoreRules, MemoryStamp};
    use crate::domain::git_worktree::GitIgnoreSource;
    use std::fs;

    fn rules(files: &[(&str, &str)]) -> (tempfile::TempDir, IgnoreRules) {
        let dir = tempfile::tempdir().expect("tempdir");
        let sources = files
            .iter()
            .map(|(base, text)| {
                let folder = dir.path().join(base);
                fs::create_dir_all(&folder).expect("folder");
                let file = folder.join(".gitignore");
                fs::write(&file, text).expect("rules");
                GitIgnoreSource {
                    file,
                    base: (*base).to_owned(),
                }
            })
            .collect::<Vec<_>>();
        let rules = IgnoreRules::read(&sources, false);
        (dir, rules)
    }

    fn memory<'a>(paths: &[&'a str]) -> Vec<&'a str> {
        paths.to_vec()
    }

    /// Запись проверки отвечает только на тот же вход: другая память, другой корень или другой
    /// файл правил — записи нет. Без `workPath` запись не создаётся (#645).
    #[test]
    fn a_check_record_answers_only_the_same_input() {
        let dir = tempfile::tempdir().expect("tempdir");
        let rules = dir.path().join(".gitignore");
        fs::write(
            &rules, "*.log
",
        )
        .expect("rules");
        let sources = [GitIgnoreSource {
            file: rules.clone(),
            base: String::new(),
        }];
        let remembered = MemoryStamp::Remembered {
            identity: None,
            generation: 7,
            files: 3,
        };
        let (input, settled) = CheckInput::of(&sources, "src/cf", &[], false, remembered.clone());
        assert!(!settled, "a file written just now is not settled");
        let work = dir.path().join("work");
        let record = CheckRecord::at(&work, &dir.path().join("memory.redb"));
        record.keep(&input);
        assert!(!record.holds(&input), "no workPath, no record");

        fs::create_dir_all(&work).expect("work");
        record.keep(&input);
        assert!(record.holds(&input));
        assert!(!record.holds(&input.clone().with_memory(MemoryStamp::Samples)));
        let next = MemoryStamp::Remembered {
            identity: None,
            generation: 8,
            files: 3,
        };
        assert!(!record.holds(&input.clone().with_memory(next)));
        let (other_root, _) = CheckInput::of(&sources, "src/other", &[], false, remembered.clone());
        assert!(!record.holds(&other_root));
        fs::write(
            &rules,
            "*.log
*.tmp
",
        )
        .expect("rules");
        let (edited, _) = CheckInput::of(&sources, "src/cf", &[], false, remembered);
        assert!(!record.holds(&edited));
    }

    #[test]
    fn a_rule_that_meets_no_dumped_file_hides_nothing() {
        let (_dir, rules) = rules(&[("", "*.log\n")]);
        assert_eq!(
            rules.hiding("src/cf", dump_samples().iter().map(String::as_str)),
            None
        );
        assert_eq!(
            rules.hiding(
                "src/cf",
                memory(&["Configuration.xml", "Ext/ParentConfigurations.bin"])
            ),
            None
        );
    }

    #[test]
    fn an_unanchored_rule_hides_a_remembered_file_at_any_depth() {
        let (_dir, rules) = rules(&[("", "*.bin\n")]);
        let hidden = rules
            .hiding(
                "src/cf",
                memory(&["Configuration.xml", "Ext/ParentConfigurations.bin"]),
            )
            .expect("hidden");
        assert_eq!(hidden.rule, "*.bin");
        assert_eq!(hidden.line, 1);
        assert_eq!(hidden.paths, ["Ext/ParentConfigurations.bin"]);
    }

    #[test]
    fn an_unanchored_directory_rule_hides_the_folders_of_a_dump() {
        let (_dir, rules) = rules(&[("", "# comment\n\nCommonModules/\n")]);
        let hidden = rules
            .hiding("src/cf", memory(&["CommonModules/Common/Ext/Module.bsl"]))
            .expect("hidden");
        assert_eq!((hidden.rule.as_str(), hidden.line), ("CommonModules/", 3));
        assert!(rules
            .hiding("src/cf", dump_samples().iter().map(String::as_str))
            .is_some());
    }

    #[test]
    fn without_memory_the_rule_is_tried_on_the_files_a_dump_writes() {
        let (_dir, bsl) = rules(&[("", "*.bsl\n")]);
        assert!(bsl
            .hiding("src/cf", dump_samples().iter().map(String::as_str))
            .is_some());
        let (_dir, log) = rules(&[("", "*.log\n")]);
        assert_eq!(
            log.hiding("src/cf", dump_samples().iter().map(String::as_str)),
            None
        );
    }

    #[test]
    fn a_rule_anchored_outside_the_root_hides_nothing() {
        let (_dir, rules) = rules(&[("", "/build/\n/docs/*.xml\n")]);
        assert_eq!(
            rules.hiding("src/cf", dump_samples().iter().map(String::as_str)),
            None
        );
    }

    #[test]
    fn a_rule_anchored_above_the_root_hides_the_root() {
        let (_dir, rules) = rules(&[("", "/src/\n")]);
        let hidden = rules
            .hiding("src/cf", memory(&["Configuration.xml"]))
            .expect("root");
        assert_eq!(hidden.paths, ["."]);
    }

    #[test]
    fn a_negation_lets_the_set_through() {
        let (_dir, rules) = rules(&[("", "/*\n!/src/\n*.bsl\n!*.bsl\n")]);
        assert_eq!(
            rules.hiding("src/cf", dump_samples().iter().map(String::as_str)),
            None
        );
    }

    #[test]
    fn a_rule_of_a_gitignore_inside_the_set_counts_from_its_folder() {
        let (_dir, rules) = rules(&[("src/cf/Catalogs", "Items/\n"), ("other", "*.xml\n")]);
        let hidden = rules
            .hiding(
                "src/cf",
                memory(&[
                    "Catalogs/Items/Ext/ObjectModule.bsl",
                    "Documents/Items/Ext/ObjectModule.bsl",
                ]),
            )
            .expect("hidden");
        assert_eq!(hidden.paths, ["Catalogs/Items/Ext/ObjectModule.bsl"]);
    }

    #[test]
    fn the_version_file_rule_hides_nothing_the_runner_reads() {
        let (_dir, rules) = rules(&[("", "ConfigDumpInfo.xml\n")]);
        assert_eq!(
            rules.hiding("src/cf", dump_samples().iter().map(String::as_str)),
            None
        );
        assert_eq!(rules.hiding("src/cf", memory(&["Configuration.xml"])), None);
    }

    /// Класс POSIX и отрицательный класс читаются, как у git: отрицательный класс не
    /// совпадает с разделителем каталогов.
    #[test]
    fn bracket_classes_read_as_git_reads_them() {
        let (_dir, posix) = rules(&[("", "[[:digit:]]*.bsl\n")]);
        assert!(posix.hiding("src/cf", memory(&["1Module.bsl"])).is_some());
        assert!(posix.hiding("src/cf", memory(&["Module.bsl"])).is_none());
        let (_dir, negated) = rules(&[("", "/src/cf[!x]Module.bsl\n")]);
        assert!(negated.hiding("src", memory(&["cf/Module.bsl"])).is_none());
        assert!(negated.hiding("src", memory(&["cfyModule.bsl"])).is_some());
    }

    #[test]
    fn a_double_star_crosses_folders() {
        let (_dir, rules) = rules(&[("", "src/**/Ext/*.bin\n")]);
        assert!(rules
            .hiding("src/cf", memory(&["Catalogs/A/Ext/Picture.bin"]))
            .is_some());
    }
}
