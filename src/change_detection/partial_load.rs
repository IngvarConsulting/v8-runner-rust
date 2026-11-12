use crate::change_detection::runner_dirs::{runner_dirs_inside, RunnerDirs};
use std::collections::HashSet;
use std::io::{self, ErrorKind};
use std::path::{Component, Path, PathBuf};

use crate::change_detection::analyzer::{ChangeKind, FileChange};
use crate::change_detection::scanner::{self, ScanScope};
use crate::support::path::{
    canonical_caseless, is_safe_path_segment, nearest_existing_canonical_path,
};
use crate::support::source_descriptor::designer_child_objects;

pub(crate) const CONFIGURATION_XML: &str = "Configuration.xml";

// Only top-level Designer metadata directories. In particular, Ext contains
// configuration payloads whose disappearance a root-only load cannot express.
const METADATA_TYPES: &[(&str, &str)] = &[
    ("AccountingRegisters", "AccountingRegister"),
    ("AccumulationRegisters", "AccumulationRegister"),
    ("Bots", "Bot"),
    ("BusinessProcesses", "BusinessProcess"),
    ("CalculationRegisters", "CalculationRegister"),
    ("Catalogs", "Catalog"),
    ("ChartsOfAccounts", "ChartOfAccounts"),
    ("ChartsOfCalculationTypes", "ChartOfCalculationTypes"),
    ("ChartsOfCharacteristicTypes", "ChartOfCharacteristicTypes"),
    ("CommandGroups", "CommandGroup"),
    ("CommonAttributes", "CommonAttribute"),
    ("CommonCommands", "CommonCommand"),
    ("CommonForms", "CommonForm"),
    ("CommonModules", "CommonModule"),
    ("CommonPictures", "CommonPicture"),
    ("CommonTemplates", "CommonTemplate"),
    ("Constants", "Constant"),
    ("DataProcessors", "DataProcessor"),
    ("DefinedTypes", "DefinedType"),
    ("DocumentJournals", "DocumentJournal"),
    ("DocumentNumerators", "DocumentNumerator"),
    ("Documents", "Document"),
    ("Enums", "Enum"),
    ("EventSubscriptions", "EventSubscription"),
    ("ExchangePlans", "ExchangePlan"),
    ("ExternalDataSources", "ExternalDataSource"),
    ("FilterCriteria", "FilterCriterion"),
    ("FunctionalOptions", "FunctionalOption"),
    ("FunctionalOptionsParameters", "FunctionalOptionsParameter"),
    ("HTTPServices", "HTTPService"),
    ("InformationRegisters", "InformationRegister"),
    ("IntegrationServices", "IntegrationService"),
    ("Languages", "Language"),
    ("Reports", "Report"),
    ("Roles", "Role"),
    ("ScheduledJobs", "ScheduledJob"),
    ("Sequences", "Sequence"),
    ("SessionParameters", "SessionParameter"),
    ("SettingsStorages", "SettingsStorage"),
    ("StyleItems", "StyleItem"),
    ("Styles", "Style"),
    ("Subsystems", "Subsystem"),
    ("Tasks", "Task"),
    ("WebServices", "WebService"),
    ("WebSocketClients", "WebSocketClient"),
    ("WSReferences", "WSReference"),
    ("XDTOPackages", "XDTOPackage"),
];

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// Decision made by [`decide`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadDecision {
    /// Load only the listed files.
    Partial(Vec<PathBuf>),
    /// Load the entire source-set directory.
    Full,
}

/// Decide whether a partial or full load is appropriate for `changes`.
pub fn decide(changes: &[FileChange], source_root: &Path) -> LoadDecision {
    decide_in_scope(changes, source_root, &[], ScanScope::Push)
}

/// Список частичной загрузки сборки: каталоги раннера под корнем в него не входят.
pub(crate) fn decide_build(
    changes: &[FileChange],
    source_root: &Path,
    dirs: RunnerDirs<'_>,
) -> LoadDecision {
    let excluded = runner_dirs_inside(source_root, dirs);
    decide_in_scope(changes, source_root, &excluded, ScanScope::Build)
}

fn decide_in_scope(
    changes: &[FileChange],
    source_root: &Path,
    excluded: &[PathBuf],
    scope: ScanScope,
) -> LoadDecision {
    match expand_files(changes, source_root, excluded, scope) {
        Some(paths) if !paths.is_empty() => LoadDecision::Partial(paths),
        _ => LoadDecision::Full,
    }
}

/// Write a partial-load list file as UTF-8 with BOM and CRLF-separated paths.
///
/// Paths are written relative to `source_root` as required by Designer's
/// `-listFile` parameter when running in agent mode. Path component separators
/// remain native to the current operating system.
pub fn write_list_file(paths: &[PathBuf], source_root: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::write(dest, list_file_bytes(paths, source_root)?)
}

/// The same list as bytes, for callers that deliver it themselves (an agent's exchange).
pub fn list_file_bytes(paths: &[PathBuf], source_root: &Path) -> std::io::Result<Vec<u8>> {
    let rel_paths = relative_paths(paths, source_root)?;
    let lines = rel_paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    let contents = lines.join("\r\n");
    let mut payload = Vec::with_capacity(UTF8_BOM.len() + contents.len());
    payload.extend_from_slice(UTF8_BOM);
    payload.extend_from_slice(contents.as_bytes());
    Ok(payload)
}

/// Convert safe absolute paths into relative paths under `source_root`.
pub fn relative_paths(paths: &[PathBuf], source_root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let root_real = canonicalize_existing(source_root).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "source root does not exist or is not canonicalizable: {}",
                source_root.display()
            ),
        )
    })?;
    let mut rel_paths = Vec::new();

    for path in paths {
        let rel = safe_relative_path(path, source_root, &root_real).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "path cannot be safely represented in partial list: {}",
                    path.display()
                ),
            )
        })?;
        if !rel.as_os_str().is_empty() {
            rel_paths.push(rel);
        }
    }

    Ok(rel_paths)
}

/// Files the agent may need in addition to a partial list that explicitly names
/// Configuration.xml. The platform reads declared objects from the source
/// directory even when their files do not appear in the partial list.
pub(crate) fn root_dependencies(source_root: &Path) -> io::Result<Vec<PathBuf>> {
    let root_real = std::fs::canonicalize(source_root)?;
    let invalid = |path: &Path| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!("cannot stage partial root dependency '{}'", path.display()),
        )
    };
    let configuration = source_root.join(CONFIGURATION_XML);
    safe_relative_path(&configuration, source_root, &root_real)
        .ok_or_else(|| invalid(&configuration))?;
    let objects = designer_child_objects(&std::fs::read_to_string(&configuration)?)
        .ok_or_else(|| invalid(&configuration))?;
    let mut paths = Vec::new();
    for (kind, name) in objects {
        let directory = METADATA_TYPES
            .iter()
            .find_map(|(directory, tag)| (*tag == kind).then_some(directory))
            .ok_or_else(|| {
                io::Error::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "cannot stage unrecognized Designer metadata type '{kind}'; use push <SET> --full"
                    ),
                )
            })?;
        if !is_safe_path_segment(&name) || name.ends_with(['.', ' ']) {
            return Err(invalid(&configuration));
        }
        let group = source_root.join(directory);
        let descriptor = group.join(format!("{name}.xml"));
        push_file_if_safe(&mut paths, &descriptor, source_root, &root_real)
            .ok_or_else(|| invalid(&descriptor))?;
        let payload = group.join(name);
        expand_optional_directory(
            &mut paths,
            &payload,
            source_root,
            &root_real,
            &[],
            ScanScope::Push,
        )
        .ok_or_else(|| invalid(&payload))?;
    }
    let root_payload = source_root.join("Ext");
    expand_optional_directory(
        &mut paths,
        &root_payload,
        source_root,
        &root_real,
        &[],
        ScanScope::Push,
    )
    .ok_or_else(|| invalid(&root_payload))?;
    Ok(paths)
}

fn expand_files(
    changes: &[FileChange],
    source_root: &Path,
    excluded: &[PathBuf],
    scope: ScanScope,
) -> Option<Vec<PathBuf>> {
    let root_real = canonicalize_existing(source_root)?;
    let relative_changes = changes
        .iter()
        .map(|change| {
            let relative = if change.kind == ChangeKind::Deleted {
                safe_deleted_path(&change.path, source_root, &root_real)?
            } else {
                safe_relative_path(&change.path, source_root, &root_real)?
            };
            Some((change, relative))
        })
        .collect::<Option<Vec<_>>>()?;

    let configuration = source_root.join(CONFIGURATION_XML);
    let mut paths = Vec::new();
    let mut needs_root = false;
    for (change, relative) in &relative_changes {
        if change.kind == ChangeKind::Deleted {
            continue;
        }

        let path = source_root.join(relative);
        if path.is_dir() {
            let before = paths.len();
            expand_directory(&mut paths, &path, source_root, &root_real, excluded, scope)?;
            if paths.len() == before {
                return None;
            }
            for descriptor in ancestor_xml_descriptors(Some(&path), source_root) {
                push_file_if_safe_if_exists(&mut paths, &descriptor, source_root, &root_real)?;
            }
            needs_root = true;
        } else {
            if change.kind == ChangeKind::Added && top_level_descriptor(relative).is_some() {
                needs_root = true;
                // A previously orphaned subtree can already be in the file snapshot.
                // The new object still needs all of its sources, including on SFTP.
                let object_dir = path.with_extension("");
                expand_optional_directory(
                    &mut paths,
                    &object_dir,
                    source_root,
                    &root_real,
                    excluded,
                    scope,
                )?;
            }
            let descriptors = ModuleDescriptors::of(change.kind, relative);
            push_with_descriptors(&mut paths, path, descriptors, source_root, &root_real)?;
        }
    }

    if needs_root {
        push_file_if_safe(&mut paths, &configuration, source_root, &root_real)?;
    }

    if changes
        .iter()
        .any(|change| change.kind == ChangeKind::Deleted)
    {
        validate_removals(&relative_changes, &configuration, &paths, source_root)?;
    }

    paths.sort_unstable();
    paths.dedup();
    Some(paths)
}

fn expand_optional_directory(
    paths: &mut Vec<PathBuf>,
    directory: &Path,
    source_root: &Path,
    root_real: &Path,
    excluded: &[PathBuf],
    scope: ScanScope,
) -> Option<()> {
    match std::fs::symlink_metadata(directory) {
        Ok(_) => expand_directory(paths, directory, source_root, root_real, excluded, scope),
        Err(error) if error.kind() == ErrorKind::NotFound => Some(()),
        Err(_) => None,
    }
}

fn expand_directory(
    paths: &mut Vec<PathBuf>,
    directory: &Path,
    source_root: &Path,
    root_real: &Path,
    excluded: &[PathBuf],
    scope: ScanScope,
) -> Option<()> {
    if excluded
        .iter()
        .any(|excluded| directory.starts_with(excluded))
    {
        return Some(());
    }
    // The scanner does not select symlink directories within a source set.
    if directory != source_root && !std::fs::symlink_metadata(directory).ok()?.is_dir() {
        return None;
    }
    for entry in scanner::source_entries(directory, excluded, scope) {
        let entry = entry.ok()?;
        let relative = safe_relative_path(entry.path(), source_root, root_real)?;
        if scanner::is_source_file(&entry, source_root, scope) {
            paths.push(source_root.join(relative));
        }
    }
    Some(())
}

// A root-only load may silently retain a declared object with a missing XML,
// or retain absent payloads of an existing object. Admit only whole removals.
fn validate_removals(
    changes: &[(&FileChange, PathBuf)],
    configuration: &Path,
    selected: &[PathBuf],
    source_root: &Path,
) -> Option<()> {
    if !changes.iter().any(|(change, path)| {
        change.kind == ChangeKind::Modified && path == Path::new(CONFIGURATION_XML)
    }) {
        return None;
    }

    let retained: HashSet<_> =
        designer_child_objects(&std::fs::read_to_string(configuration).ok()?)?
            .into_iter()
            .map(|(kind, name)| (kind, canonical_caseless(&name)))
            .collect();
    let mut removed = HashSet::new();
    for (change, path) in changes {
        if change.kind == ChangeKind::Deleted {
            if let Some((kind, name)) = top_level_descriptor(path) {
                let name = canonical_caseless(name);
                if retained.contains(&(kind.to_owned(), name.clone())) {
                    return None;
                }
                removed.insert((kind, name));
            }
        }
    }

    let belongs_to_removed = |path: &Path| {
        top_level_object(path).is_some_and(|(directory, name)| {
            removed.contains(&(directory, canonical_caseless(name)))
        })
    };
    if changes
        .iter()
        .any(|(change, path)| change.kind == ChangeKind::Deleted && !belongs_to_removed(path))
    {
        return None;
    }
    for path in selected {
        if belongs_to_removed(path.strip_prefix(source_root).ok()?) {
            return None;
        }
    }
    Some(())
}

/// Elements a Designer dump lays out under an object's directory: the folder, the kind the
/// element's full name carries (`Catalog.X.Form.F`) and, when only one kind owns them, that
/// owner's kind. A nested subsystem lies in the top-level folder of its kind
/// ([`METADATA_TYPES`]) under a subsystem.
const NESTED_ELEMENTS: &[(&str, &str, Option<&str>)] = &[
    ("Forms", "Form", None),
    ("Templates", "Template", None),
    ("Commands", "Command", None),
    (
        "Recalculations",
        "Recalculation",
        Some("CalculationRegister"),
    ),
];

/// The folders of a Designer dump: those of the top-level metadata kinds ([`METADATA_TYPES`])
/// and those nested under an element ([`NESTED_ELEMENTS`]). One vocabulary for the dump
/// layout, read also by the git ignore check (#575).
pub(crate) fn dump_layout_folders() -> (Vec<&'static str>, Vec<&'static str>) {
    (
        METADATA_TYPES.iter().map(|(folder, _)| *folder).collect(),
        NESTED_ELEMENTS
            .iter()
            .map(|(folder, _, _)| *folder)
            .collect(),
    )
}

/// The kind of a folder nested under an element of `owner_kind`, if a dump lays one out there.
fn nested_kind(owner_kind: &str, folder: &str) -> Option<&'static str> {
    let nested = NESTED_ELEMENTS.iter().find_map(|(directory, kind, owner)| {
        (directory.eq_ignore_ascii_case(folder) && owner.is_none_or(|owner| owner == owner_kind))
            .then_some(*kind)
    });
    nested.or_else(|| {
        METADATA_TYPES
            .iter()
            .find(|(directory, kind)| {
                *kind == "Subsystem"
                    && owner_kind == "Subsystem"
                    && directory.eq_ignore_ascii_case(folder)
            })
            .map(|(_, kind)| *kind)
    })
}

/// The deepest element of a Designer dump that owns a file, as [`dump_element`] finds it.
struct DumpElement {
    /// Kind of the deepest element as `ConfigDumpInfo.xml` spells it: `CommonModule`, `Form`.
    kind: &'static str,
    /// Full name of the element: `Catalog.X.Form.F`.
    name: String,
    /// How many leading path components name the element; the rest lies inside its folder.
    folders: usize,
}

/// The deepest element of a Designer dump that owns the file at `path`, relative to the dump
/// root, named as the version file `ConfigDumpInfo.xml` names it: `Catalog.X` for
/// `Catalogs/X.xml` and everything under `Catalogs/X/`, `Catalog.X.Form.F` under `Forms/F.xml`
/// and `Forms/F/`, likewise `Template` (`Templates/`), `Command` (only `Commands/C/`),
/// `Recalculation` of a calculation register (`Recalculations/`) and a nested `Subsystem`
/// (`Subsystems/`, at any depth). `None` when the top-level folder is not a kind of
/// [`METADATA_TYPES`] (`Configuration.xml`, `Ext/`, a stray file).
pub(crate) fn dump_element_of(path: &Path) -> Option<String> {
    dump_element(path).map(|element| element.name)
}

/// The one walk over a dump path behind [`dump_element_of`] and [`module_loads_alone`].
fn dump_element(path: &Path) -> Option<DumpElement> {
    let (kind, name) = top_level_object(path)?;
    let mut element = DumpElement {
        kind,
        name: format!("{kind}.{name}"),
        folders: 2,
    };
    let mut rest = path.iter().skip(2).peekable();
    while let Some(folder) = rest.next().and_then(|part| part.to_str()) {
        let Some(nested) = nested_kind(element.kind, folder) else {
            break;
        };
        let Some(entry) = rest.next() else {
            break;
        };
        let child = if rest.peek().is_some() {
            entry.to_str()
        } else {
            // A file right in the folder is the element's descriptor `<Name>.xml`; a command
            // has none.
            let file = Path::new(entry);
            file.extension()
                .and_then(|extension| extension.to_str())
                .filter(|extension| extension.eq_ignore_ascii_case("xml") && nested != "Command")
                .and_then(|_| file.file_stem()?.to_str())
        };
        let Some(child) = child else {
            break;
        };
        element.name = format!("{}.{nested}.{child}", element.name);
        element.kind = nested;
        element.folders += 2;
    }
    Some(element)
}

fn top_level_descriptor(path: &Path) -> Option<(&'static str, &str)> {
    (path.components().count() == 2).then_some(())?;
    top_level_object(path)
}

fn top_level_object(path: &Path) -> Option<(&'static str, &str)> {
    let mut parts = path.iter();
    let folder = parts.next()?.to_str()?;
    let kind = METADATA_TYPES
        .iter()
        .find_map(|(directory, kind)| directory.eq_ignore_ascii_case(folder).then_some(*kind))?;
    let owner = parts.next()?;
    let name = if parts.next().is_some() {
        owner.to_str()?
    } else {
        let file = Path::new(owner);
        if !file.extension()?.to_str()?.eq_ignore_ascii_case("xml") {
            return None;
        }
        file.file_stem()?.to_str()?
    };
    Some((kind, name))
}

/// Whether a changed `.bsl` goes to the list with the descriptors of its owners.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModuleDescriptors {
    /// The sibling `<module>.xml` and every existing descriptor up the tree go with it.
    Owners,
    /// The module goes alone (`INV.USE-CASES.A-CHANGED-MODULE-LOADS-WITHOUT-ITS-DESCRIPTION`).
    Omitted,
}

impl ModuleDescriptors {
    fn of(change: ChangeKind, relative: &Path) -> Self {
        if change == ChangeKind::Modified && module_loads_alone(relative) {
            Self::Omitted
        } else {
            Self::Owners
        }
    }
}

/// A changed module the platform loads as a separate file, without its owner's descriptors
/// (`INV.USE-CASES.A-CHANGED-MODULE-LOADS-WITHOUT-ITS-DESCRIPTION`): the module of a common
/// module (`CommonModules/<M>/Ext/Module.bsl`), a form module
/// (`.../Forms/<F>/Ext/Form/Module.bsl`, `CommonForms/<F>/Ext/Form/Module.bsl`) and the object
/// module of a top-level object (`Catalogs/<C>/Ext/ObjectModule.bsl`). The platform applies
/// each of them alone (measured on КА). Manager, record set, value manager and command
/// modules keep their descriptors until a measurement shows the platform applies them alone.
fn module_loads_alone(relative: &Path) -> bool {
    let Some(element) = dump_element(relative) else {
        return false;
    };
    let Some(inside) = relative
        .iter()
        .skip(element.folders)
        .map(|part| part.to_str())
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };
    let is = |part: &str, name: &str| part.eq_ignore_ascii_case(name);
    // The tail is counted inside the deepest element, not along the whole path: a common
    // module is `CommonModules/<M>` plus `Ext/Module.bsl` (four parts in all), a form is its
    // own folders plus `Ext/Form/Module.bsl` — five parts for a common form, seven for a form
    // of an object. Counting from the element keeps `Catalogs/Form/Ext/Form/Module.bsl` (a
    // catalog named `Form`) and an object command's `Ext/...` out of the form branch. An object
    // module belongs to a top-level object only: `<Group>/<Name>` plus `Ext/ObjectModule.bsl`.
    match (element.kind, inside.as_slice()) {
        ("CommonModule", [ext, module]) => is(ext, "Ext") && is(module, "Module.bsl"),
        (_, [ext, module]) if element.folders == 2 => {
            is(ext, "Ext") && is(module, "ObjectModule.bsl")
        }
        ("CommonForm" | "Form", [ext, folder, module]) => {
            is(ext, "Ext") && is(folder, "Form") && is(module, "Module.bsl")
        }
        _ => false,
    }
}

/// `paths` of a decided partial load with the owners' descriptors of every `.bsl` added back:
/// the list for a channel where a module alone is not measured yet. The SFTP exchange of the
/// agent loads modules with their descriptors, as before #599, until a live run shows the
/// platform applies a lone module there too
/// (`INV.USE-CASES.A-CHANGED-MODULE-LOADS-WITHOUT-ITS-DESCRIPTION`).
pub(crate) fn with_module_descriptors(
    paths: &[PathBuf],
    source_root: &Path,
) -> Result<Vec<PathBuf>, DescriptorWidening> {
    let root_real = canonicalize_existing(source_root)
        .ok_or_else(|| DescriptorWidening::Missing(source_root.to_path_buf()))?;
    let mut widened = Vec::with_capacity(paths.len());
    for path in paths {
        // The planned files have passed the path guard; one that is gone now was removed
        // between planning and staging.
        if !path.is_file() {
            return Err(DescriptorWidening::Missing(path.clone()));
        }
        if is_bsl(path) {
            for descriptor in owner_descriptors(path, source_root) {
                push_file_if_safe_if_exists(&mut widened, &descriptor, source_root, &root_real)
                    .ok_or(DescriptorWidening::Rejected(descriptor))?;
            }
        }
        widened.push(path.clone());
    }
    widened.sort_unstable();
    widened.dedup();
    Ok(widened)
}

/// Why [`with_module_descriptors`] could not widen a planned partial load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DescriptorWidening {
    /// The planned file (or the source root) is gone: removed between planning and staging.
    Missing(PathBuf),
    /// An owner's descriptor failed the path guard: outside the source root or not a file.
    Rejected(PathBuf),
}

fn push_with_descriptors(
    paths: &mut Vec<PathBuf>,
    path: PathBuf,
    descriptors: ModuleDescriptors,
    source_root: &Path,
    root_real: &Path,
) -> Option<()> {
    // Both direct changes and expanded entries have already passed the path guard.
    if !path.is_file() {
        return None;
    }
    if is_bsl(&path) && descriptors == ModuleDescriptors::Owners {
        for descriptor in owner_descriptors(&path, source_root) {
            push_file_if_safe_if_exists(paths, &descriptor, source_root, root_real)?;
        }
    }
    paths.push(path);
    Some(())
}

/// The sibling `<module>.xml` of a `.bsl` and every descriptor up the tree, existing or not.
fn owner_descriptors(module: &Path, source_root: &Path) -> impl Iterator<Item = PathBuf> {
    sibling_xml(module)
        .into_iter()
        .chain(ancestor_xml_descriptors(module.parent(), source_root))
}

fn push_file_if_safe(
    paths: &mut Vec<PathBuf>,
    candidate: &Path,
    source_root: &Path,
    root_real: &Path,
) -> Option<()> {
    if !candidate.is_file() {
        return None;
    }
    let relative = safe_relative_path(candidate, source_root, root_real)?;
    paths.push(source_root.join(relative));
    Some(())
}

fn push_file_if_safe_if_exists(
    paths: &mut Vec<PathBuf>,
    candidate: &Path,
    source_root: &Path,
    root_real: &Path,
) -> Option<()> {
    match std::fs::symlink_metadata(candidate) {
        Ok(_) => push_file_if_safe(paths, candidate, source_root, root_real),
        Err(error) if error.kind() == ErrorKind::NotFound => Some(()),
        Err(_) => None,
    }
}

fn safe_relative_path(path: &Path, source_root: &Path, root_real: &Path) -> Option<PathBuf> {
    if path.components().any(|part| part == Component::ParentDir) {
        return None;
    }
    let candidate_real = canonicalize_existing(path)?;
    let canonical_relative = candidate_real.strip_prefix(root_real).ok()?;
    let relative = path.strip_prefix(source_root).unwrap_or(canonical_relative);
    is_representable(relative).then(|| relative.to_path_buf())
}

fn safe_deleted_path(path: &Path, source_root: &Path, root_real: &Path) -> Option<PathBuf> {
    let relative = path.strip_prefix(source_root).ok()?;
    if !is_representable(relative)
        || !std::fs::symlink_metadata(path).is_err_and(|error| error.kind() == ErrorKind::NotFound)
    {
        return None;
    }
    // A missing file can still be reached through an escaping or dangling link.
    nearest_existing_canonical_path(path)
        .ok()?
        .starts_with(root_real)
        .then(|| relative.to_path_buf())
}

fn is_representable(relative: &Path) -> bool {
    // SFTP uses slash-separated paths, list files use CRLF, and ibcmd takes
    // positional arguments alongside options. Reject ambiguous encodings.
    !relative.to_str().is_some_and(|path| path.starts_with('-'))
        && relative.components().all(|part| {
            matches!(part, Component::Normal(name)
                if name.to_str().is_some_and(|name| is_safe_path_segment(name) && !name.ends_with(['.', ' '])))
        })
}

fn canonicalize_existing(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

/// Что частичная загрузка списка `paths` (из [`decide_build`]) оставит следующей выгрузке по
/// прежней описи (#646): сколько модулей и прочих файлов она выгрузит обратно и не уйдёт ли
/// она в полную.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct LoadedForDump {
    /// Выгрузка по такой описи будет полной: в списке корневой `Configuration.xml` (удалённый,
    /// переименованный или добавленный объект, свойства конфигурации — у удаления и
    /// переименования он в списке всегда, частичная загрузка без него их не принимает) или
    /// `Ext/Predefined.xml`, `Ext/Flowchart.xml` (документация платформы: такие изменения
    /// выгружаются полностью).
    pub(crate) full_dump: bool,
    /// Модули `.bsl`.
    pub(crate) modules: usize,
    /// Остальные файлы: описания объектов, форм, макеты, справка.
    pub(crate) other: usize,
}

/// [`LoadedForDump`] списка частичной загрузки `paths` под корнем `source_root`.
pub(crate) fn loaded_for_dump(paths: &[PathBuf], source_root: &Path) -> LoadedForDump {
    paths
        .iter()
        .fold(LoadedForDump::default(), |mut loaded, path| {
            let relative = path.strip_prefix(source_root).unwrap_or(path);
            loaded.full_dump |= forces_a_full_dump(relative);
            if is_bsl(relative) {
                loaded.modules += 1;
            } else {
                loaded.other += 1;
            }
            loaded
        })
}

fn forces_a_full_dump(relative: &Path) -> bool {
    if relative == Path::new(CONFIGURATION_XML) {
        return true;
    }
    let in_ext = relative
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|folder| folder.eq_ignore_ascii_case("Ext"));
    in_ext
        && relative.file_name().is_some_and(|name| {
            name.eq_ignore_ascii_case("Predefined.xml")
                || name.eq_ignore_ascii_case("Flowchart.xml")
        })
}

/// Part of the element a file under its own `Ext` directory belongs to, as the version file
/// names it: `Catalogs/X/Ext/ObjectModule.bsl` — `Catalog.X.ObjectModule`, `Ext/Help.xml` and
/// `Ext/Help/…` — `<element>.Help`, a form's `Forms/F/Ext/Form.xml` and `Forms/F/Ext/Form/Module.bsl`
/// — `Catalog.X.Form.F.Form`. `Ext` counts only right under the element's folder. Such a part
/// drops out of the version file when only it is removed and the element stays: a dump over the
/// tree removed its files itself (#646, measured on 8.3.27). A name the version file never had
/// matches nothing.
pub(crate) fn part_of_dump_element(path: &Path) -> Option<String> {
    let element = dump_element(path)?;
    let mut rest = path.iter().skip(element.folders);
    rest.next()
        .filter(|folder| folder.eq_ignore_ascii_case("Ext"))?;
    let part = Path::new(rest.next()?).file_stem()?.to_str()?;
    Some(format!("{}.{part}", element.name))
}

fn is_bsl(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("bsl"))
        .unwrap_or(false)
}

/// Return the XML descriptor alongside a `.bsl` file (same name, `.xml` ext).
fn sibling_xml(bsl: &Path) -> Option<PathBuf> {
    let parent = bsl.parent()?;
    let stem = bsl.file_stem()?.to_str()?;
    Some(parent.join(format!("{stem}.xml")))
}

fn ancestor_xml_descriptors(mut current: Option<&Path>, source_root: &Path) -> Vec<PathBuf> {
    let mut descriptors = Vec::new();

    while let Some(dir) = current {
        if dir == source_root {
            break;
        }

        descriptors.push(dir.with_extension("xml"));
        current = dir.parent();
    }

    descriptors
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::{
        decide, decide_build, dump_element_of, loaded_for_dump, part_of_dump_element,
        relative_paths, root_dependencies, write_list_file, LoadDecision, LoadedForDump,
        RunnerDirs,
    };
    use crate::change_detection::analyzer::{ChangeKind, FileChange};

    /// Владелец файла выгрузки — самый глубокий элемент, который раннер узнаёт по пути
    /// (`INV.USE-CASES.THE-DUMP-MODE-IS-FORECAST-IN-THE-SAME-COMMAND`).
    #[test]
    fn a_dump_file_belongs_to_the_deepest_element_its_path_names() {
        let owner = |path: &str| dump_element_of(std::path::Path::new(path));
        for (path, expected) in [
            ("Catalogs/Items.xml", Some("Catalog.Items")),
            ("catalogs/Items/Ext/ObjectModule.bsl", Some("Catalog.Items")),
            ("Catalogs/Items/Stray.txt", Some("Catalog.Items")),
            (
                "Catalogs/Items/Forms/Item.xml",
                Some("Catalog.Items.Form.Item"),
            ),
            (
                "Catalogs/Items/Forms/Item/Ext/Form/Module.bsl",
                Some("Catalog.Items.Form.Item"),
            ),
            (
                "Documents/Order/Templates/Print/Ext/Template.xml",
                Some("Document.Order.Template.Print"),
            ),
            (
                "Catalogs/Items/Commands/Open/Ext/CommandModule.bsl",
                Some("Catalog.Items.Command.Open"),
            ),
            ("Catalogs/Items/Commands/Open.xml", Some("Catalog.Items")),
            (
                "Subsystems/Sales/Subsystems/Orders/Subsystems/Lines.xml",
                Some("Subsystem.Sales.Subsystem.Orders.Subsystem.Lines"),
            ),
            ("Catalogs/Items/Subsystems/Other.xml", Some("Catalog.Items")),
            (
                "CalculationRegisters/Pay/Recalculations/Again.xml",
                Some("CalculationRegister.Pay.Recalculation.Again"),
            ),
            (
                "CalculationRegisters/Pay/Recalculations/Again/Ext/RecordSetModule.bsl",
                Some("CalculationRegister.Pay.Recalculation.Again"),
            ),
            (
                "Catalogs/Items/Recalculations/Again.xml",
                Some("Catalog.Items"),
            ),
            ("CommonForms/Form/Ext/Form.xml", Some("CommonForm.Form")),
            ("Configuration.xml", None),
            ("Ext/ParentConfigurations.bin", None),
            ("Unknowns/Thing.xml", None),
        ] {
            assert_eq!(owner(path).as_deref(), expected, "{path}");
        }
    }

    #[test]
    fn a_part_is_named_only_under_the_ext_folder_of_its_element() {
        for (path, part) in [
            (
                "Catalogs/X/Ext/ObjectModule.bsl",
                Some("Catalog.X.ObjectModule"),
            ),
            ("Catalogs/X/Ext/Help/ru.html", Some("Catalog.X.Help")),
            (
                "Catalogs/X/Forms/F/Ext/Form/Module.bsl",
                Some("Catalog.X.Form.F.Form"),
            ),
            (
                "CommonModules/M/Ext/Module.bsl",
                Some("CommonModule.M.Module"),
            ),
            ("Catalogs/X.xml", None),
            ("Catalogs/X/Forms/F.xml", None),
            // `Ext` deeper than the element's own folder is not its part.
            ("Catalogs/X/Ext/Help/Ext/ru.html", Some("Catalog.X.Help")),
            (
                "Catalogs/X/Templates/T/Ext/Template.bin",
                Some("Catalog.X.Template.T.Template"),
            ),
            ("Configuration.xml", None),
            ("Ext/ManagedApplicationModule.bsl", None),
        ] {
            assert_eq!(
                part_of_dump_element(std::path::Path::new(path)).as_deref(),
                part,
                "{path}"
            );
        }
    }

    #[test]
    fn a_load_list_names_what_the_next_dump_takes_back() {
        let root = std::path::Path::new("/sources");
        let list =
            |paths: &[&str]| -> Vec<PathBuf> { paths.iter().map(|p| root.join(p)).collect() };

        assert_eq!(
            loaded_for_dump(
                &list(&[
                    "CommonModules/M/Ext/Module.bsl",
                    "Catalogs/X.xml",
                    "Catalogs/X/Forms/F/Ext/Form.xml",
                ]),
                root
            ),
            LoadedForDump {
                full_dump: false,
                modules: 1,
                other: 2
            }
        );
        for full in [
            "Configuration.xml",
            "Catalogs/X/Ext/Predefined.xml",
            "BusinessProcesses/P/Ext/Flowchart.xml",
            "Catalogs/X/ext/predefined.XML",
        ] {
            assert!(loaded_for_dump(&list(&[full]), root).full_dump, "{full}");
        }
        for partial in [
            "Catalogs/Configuration.xml",
            "Catalogs/X/Predefined.xml",
            "Catalogs/X/Ext/Predefined/Item.xml",
        ] {
            assert!(
                !loaded_for_dump(&list(&[partial]), root).full_dump,
                "{partial}"
            );
        }
    }

    #[test]
    fn write_list_file_skips_empty_relative_paths() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let list_file = root.join("partial.lst");

        write_list_file(&[root.to_path_buf()], root, &list_file).expect("write list");

        assert_eq!(
            std::fs::read(list_file).expect("read list"),
            b"\xEF\xBB\xBF"
        );
    }

    #[test]
    fn write_list_file_uses_utf8_bom_and_crlf_for_unicode_relative_paths() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path().join("src");
        let first = root.join("CommonModules").join("ОбщийМодуль1.xml");
        let second = root
            .join("CommonModules")
            .join("ОбщийМодуль1")
            .join("Ext")
            .join("Module.bsl");
        let list_file = temp.path().join("partial.lst");

        std::fs::create_dir_all(first.parent().expect("first parent")).expect("first parent dir");
        std::fs::create_dir_all(second.parent().expect("second parent"))
            .expect("second parent dir");
        std::fs::write(&first, "<xml />").expect("write first");
        std::fs::write(&second, "procedure Test()\nendprocedure").expect("write second");

        write_list_file(&[first.clone(), second.clone()], &root, &list_file).expect("write list");

        let relative_payload = [first, second]
            .iter()
            .map(|path| {
                path.strip_prefix(&root)
                    .expect("relative path")
                    .display()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\r\n");
        let mut expected = b"\xEF\xBB\xBF".to_vec();
        expected.extend_from_slice(relative_payload.as_bytes());

        assert_eq!(std::fs::read(list_file).expect("read list"), expected);
    }

    #[test]
    fn relative_paths_returns_relative_entries_for_safe_paths() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let nested = root.join("Catalogs.Items");
        std::fs::create_dir_all(&nested).expect("mkdir");
        let file = nested.join("ObjectModule.bsl");
        std::fs::write(&file, "module").expect("write");

        let rels = relative_paths(std::slice::from_ref(&file), root).expect("relative paths");

        assert_eq!(rels, vec![PathBuf::from("Catalogs.Items/ObjectModule.bsl")]);
    }

    #[cfg(unix)]
    #[test]
    fn relative_paths_rejects_path_outside_root() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().expect("tempdir");
        let root = temp.path().join("src");
        let outside = temp.path().join("outside");
        let link = root.join("Catalogs.Items");
        let escaped = outside.join("ObjectModule.bsl");

        std::fs::create_dir_all(&root).expect("root");
        std::fs::create_dir_all(&outside).expect("outside");
        std::fs::write(&escaped, "module").expect("escaped");
        symlink(&outside, &link).expect("link");

        let err =
            relative_paths(&[link.join("ObjectModule.bsl")], &root).expect_err("expected error");

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[cfg(unix)]
    #[test]
    fn write_list_file_fails_for_paths_outside_root() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().expect("tempdir");
        let root = temp.path().join("src");
        let outside = temp.path().join("outside");
        let link = root.join("Catalogs.Items");
        let escaped = outside.join("ObjectModule.bsl");
        let list_file = temp.path().join("partial.lst");

        std::fs::create_dir_all(&root).expect("root");
        std::fs::create_dir_all(&outside).expect("outside");
        std::fs::write(&escaped, "module").expect("escaped");
        symlink(&outside, &link).expect("link");

        let err = write_list_file(&[link.join("ObjectModule.bsl")], &root, &list_file)
            .expect_err("expected invalid path");

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn decide_expands_bsl_to_existing_xml_files_only() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let object_dir = root.join("Catalogs.Items");
        let module = object_dir.join("ObjectModule.bsl");
        let xml = object_dir.join("ObjectModule.xml");

        std::fs::create_dir_all(&object_dir).expect("create object dir");
        std::fs::write(&module, "module").expect("write module");
        std::fs::write(&xml, "<xml />").expect("write xml");

        let decision = decide(
            &[FileChange {
                path: module.clone(),
                kind: ChangeKind::Modified,
            }],
            root,
        );

        assert_eq!(decision, LoadDecision::Partial(vec![module, xml]));
    }

    fn write_source(root: &std::path::Path, name: &str, content: &str) -> PathBuf {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, content).expect("write source");
        path
    }

    fn change(path: &std::path::Path, kind: ChangeKind) -> FileChange {
        FileChange {
            path: path.to_path_buf(),
            kind,
        }
    }

    fn sorted(mut paths: Vec<PathBuf>) -> LoadDecision {
        paths.sort();
        LoadDecision::Partial(paths)
    }

    /// Правка одного общего модуля грузит один `.bsl`: описание модуля не менялось
    /// (`INV.USE-CASES.A-CHANGED-MODULE-LOADS-WITHOUT-ITS-DESCRIPTION`).
    #[test]
    fn a_changed_common_module_loads_without_its_description() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        write_source(root, "Configuration.xml", "<xml/>");
        write_source(root, "CommonModules/Tools.xml", "<xml/>");
        let module = write_source(root, "CommonModules/Tools/Ext/Module.bsl", "module");

        let decision = decide(&[change(&module, ChangeKind::Modified)], root);

        assert_eq!(decision, LoadDecision::Partial(vec![module]));
    }

    /// Правка модуля формы грузит только `.bsl` формы — без `Form.xml`, описания формы и
    /// описания владельца; у общей формы так же.
    #[test]
    fn a_changed_form_module_loads_without_the_form_description() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        for (module, descriptors) in [
            (
                "Catalogs/Items/Forms/ItemForm/Ext/Form/Module.bsl",
                &[
                    "Catalogs/Items/Forms/ItemForm/Ext/Form.xml",
                    "Catalogs/Items/Forms/ItemForm.xml",
                    "Catalogs/Items.xml",
                ][..],
            ),
            (
                "CommonForms/Main/Ext/Form/Module.bsl",
                &["CommonForms/Main/Ext/Form.xml", "CommonForms/Main.xml"][..],
            ),
        ] {
            for descriptor in descriptors {
                write_source(root, descriptor, "<xml/>");
            }
            let module = write_source(root, module, "module");

            let decision = decide(&[change(&module, ChangeKind::Modified)], root);

            assert_eq!(decision, LoadDecision::Partial(vec![module]));
        }
    }

    /// Изменённое описание входит в список рядом с модулем: оно изменилось само.
    #[test]
    fn a_changed_module_and_its_changed_description_load_together() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let descriptor = write_source(root, "CommonModules/Tools.xml", "<xml/>");
        let module = write_source(root, "CommonModules/Tools/Ext/Module.bsl", "module");
        let form = write_source(
            root,
            "Catalogs/Items/Forms/ItemForm/Ext/Form/Module.bsl",
            "module",
        );
        let form_layout =
            write_source(root, "Catalogs/Items/Forms/ItemForm/Ext/Form.xml", "<xml/>");
        write_source(root, "Catalogs/Items/Forms/ItemForm.xml", "<xml/>");
        write_source(root, "Catalogs/Items.xml", "<xml/>");

        let decision = decide(
            &[
                change(&descriptor, ChangeKind::Modified),
                change(&module, ChangeKind::Modified),
                change(&form, ChangeKind::Modified),
                change(&form_layout, ChangeKind::Modified),
            ],
            root,
        );

        assert_eq!(
            decision,
            sorted(vec![descriptor, module, form, form_layout])
        );
    }

    /// Новый общий модуль грузится с описанием и корнем, как любой новый объект.
    #[test]
    fn an_added_common_module_loads_with_its_description_and_the_root() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let configuration = write_source(root, "Configuration.xml", "<xml/>");
        let descriptor = write_source(root, "CommonModules/Tools.xml", "<xml/>");
        let module = write_source(root, "CommonModules/Tools/Ext/Module.bsl", "module");

        let decision = decide(
            &[
                change(&configuration, ChangeKind::Modified),
                change(&descriptor, ChangeKind::Added),
                change(&module, ChangeKind::Added),
            ],
            root,
        );

        assert_eq!(decision, sorted(vec![configuration, descriptor, module]));
    }

    /// Модуль, которого у существующего объекта не было, грузится с описаниями объекта и
    /// формы: без описаний платформа его не приняла бы — это не замерено.
    #[test]
    fn a_module_added_to_an_existing_object_loads_with_its_descriptions() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let object = write_source(root, "Catalogs/Items.xml", "<xml/>");
        let form = write_source(root, "Catalogs/Items/Forms/ItemForm.xml", "<xml/>");
        let layout = write_source(root, "Catalogs/Items/Forms/ItemForm/Ext/Form.xml", "<xml/>");
        let object_module = write_source(root, "Catalogs/Items/Ext/ObjectModule.bsl", "module");
        let form_module = write_source(
            root,
            "Catalogs/Items/Forms/ItemForm/Ext/Form/Module.bsl",
            "module",
        );
        let common = write_source(root, "CommonModules/Tools.xml", "<xml/>");
        let common_module = write_source(root, "CommonModules/Tools/Ext/Module.bsl", "module");

        assert_eq!(
            decide(&[change(&object_module, ChangeKind::Added)], root),
            sorted(vec![object.clone(), object_module])
        );
        assert_eq!(
            decide(&[change(&form_module, ChangeKind::Added)], root),
            sorted(vec![object, form, layout, form_module])
        );
        assert_eq!(
            decide(&[change(&common_module, ChangeKind::Added)], root),
            sorted(vec![common, common_module])
        );
    }

    /// Правка модуля объекта справочника или документа грузит один `.bsl`: на КА загрузка
    /// `ObjectModule.bsl` без описания справочника применилась.
    #[test]
    fn a_changed_object_module_loads_without_its_description() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        for (module, descriptor) in [
            ("Catalogs/Items/Ext/ObjectModule.bsl", "Catalogs/Items.xml"),
            (
                "Documents/Order/Ext/ObjectModule.bsl",
                "Documents/Order.xml",
            ),
        ] {
            write_source(root, descriptor, "<xml/>");
            let module = write_source(root, module, "module");

            assert_eq!(
                decide(&[change(&module, ChangeKind::Modified)], root),
                LoadDecision::Partial(vec![module.clone()]),
                "{}",
                module.display()
            );
        }
    }

    /// Модуль менеджера, набора записей и команды объекта пока грузится с описаниями
    /// владельцев: что платформа применяет их одних, не замерено.
    #[test]
    fn a_changed_manager_record_set_or_command_module_still_loads_with_its_descriptions() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        for (module, descriptor) in [
            (
                "Documents/Order/Ext/ManagerModule.bsl",
                "Documents/Order.xml",
            ),
            (
                "Documents/Order/Commands/Print/Ext/CommandModule.bsl",
                "Documents/Order.xml",
            ),
            (
                "InformationRegisters/Prices/Ext/RecordSetModule.bsl",
                "InformationRegisters/Prices.xml",
            ),
        ] {
            let descriptor = write_source(root, descriptor, "<xml/>");
            let module = write_source(root, module, "module");
            assert_eq!(
                decide(&[change(&module, ChangeKind::Modified)], root),
                sorted(vec![descriptor, module.clone()]),
                "{}",
                module.display()
            );
        }
    }

    /// Пути, похожие на модуль общего модуля или формы, но принадлежащие другому элементу,
    /// грузятся с описаниями: модуль общей команды и команды объекта, справочник с именем
    /// `Form`, форма внутри общего модуля, `ObjectModule.bsl` в каталоге формы.
    #[test]
    fn paths_that_only_look_like_a_lone_module_keep_their_descriptions() {
        for (module, descriptor) in [
            (
                "CommonCommands/Print/Ext/CommandModule.bsl",
                "CommonCommands/Print.xml",
            ),
            (
                "Catalogs/Items/Commands/Form/Ext/CommandModule.bsl",
                "Catalogs/Items.xml",
            ),
            ("Catalogs/Form/Ext/Form/Module.bsl", "Catalogs/Form.xml"),
            (
                "Catalogs/Items/Forms/ItemForm/Ext/ObjectModule.bsl",
                "Catalogs/Items.xml",
            ),
            ("Catalogs/Form/Ext/Module.bsl", "Catalogs/Form.xml"),
            (
                "CommonModules/Tools/Ext/Form/Module.bsl",
                "CommonModules/Tools.xml",
            ),
            (
                "Catalogs/CommonModules/Ext/Module.bsl",
                "Catalogs/CommonModules.xml",
            ),
        ] {
            let temp = tempdir().expect("tempdir");
            let root = temp.path();
            let descriptor = write_source(root, descriptor, "<xml/>");
            let module = write_source(root, module, "module");

            assert_eq!(
                decide(&[change(&module, ChangeKind::Modified)], root),
                sorted(vec![descriptor, module.clone()]),
                "{}",
                module.display()
            );
        }
    }

    /// Регистр букв в пути на решение не влияет, как и во всём планировщике: общий модуль,
    /// форма и модуль объекта в другом регистре грузятся одни, модуль менеджера — с описанием.
    #[test]
    fn letter_case_does_not_change_which_module_loads_alone() {
        for module in [
            "commonmodules/Tools/EXT/module.BSL",
            "COMMONFORMS/Main/ext/FORM/Module.bsl",
            "catalogs/Items/forms/ItemForm/Ext/form/MODULE.bsl",
            "DOCUMENTS/Order/ext/objectmodule.BSL",
        ] {
            let temp = tempdir().expect("tempdir");
            let root = temp.path();
            let module = write_source(root, module, "module");

            assert_eq!(
                decide(&[change(&module, ChangeKind::Modified)], root),
                LoadDecision::Partial(vec![module.clone()]),
                "{}",
                module.display()
            );
        }
        for (module, descriptor) in [
            ("catalogs/Items/EXT/managermodule.BSL", "catalogs/Items.xml"),
            ("CATALOGS/Form/ext/form/module.bsl", "CATALOGS/Form.xml"),
        ] {
            let temp = tempdir().expect("tempdir");
            let root = temp.path();
            let descriptor = write_source(root, descriptor, "<xml/>");
            let module = write_source(root, module, "module");

            assert_eq!(
                decide(&[change(&module, ChangeKind::Modified)], root),
                sorted(vec![descriptor, module.clone()]),
                "{}",
                module.display()
            );
        }
    }

    /// По SFTP модуль без описаний не замерен: список возвращает описания владельцев каждого
    /// `.bsl`, как до правила, и не трогает остальные файлы: у изменённой справки
    /// `Catalogs/Other/Ext/Help.xml` описание `Catalogs/Other.xml` в список не попадает.
    #[test]
    fn the_sftp_list_takes_the_module_descriptions_back() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let common = write_source(root, "CommonModules/Tools.xml", "<xml/>");
        let module = write_source(root, "CommonModules/Tools/Ext/Module.bsl", "module");
        let object = write_source(root, "Catalogs/Items.xml", "<xml/>");
        let form = write_source(root, "Catalogs/Items/Forms/ItemForm.xml", "<xml/>");
        let layout = write_source(root, "Catalogs/Items/Forms/ItemForm/Ext/Form.xml", "<xml/>");
        let form_module = write_source(
            root,
            "Catalogs/Items/Forms/ItemForm/Ext/Form/Module.bsl",
            "module",
        );
        write_source(root, "Catalogs/Other.xml", "<xml/>");
        let help = write_source(root, "Catalogs/Other/Ext/Help.xml", "<xml/>");
        let lone = decide(
            &[
                change(&module, ChangeKind::Modified),
                change(&form_module, ChangeKind::Modified),
                change(&help, ChangeKind::Modified),
            ],
            root,
        );
        assert_eq!(
            lone,
            sorted(vec![module.clone(), form_module.clone(), help.clone()])
        );
        let LoadDecision::Partial(lone) = lone else {
            unreachable!("asserted above")
        };

        assert_eq!(
            super::with_module_descriptors(&lone, root).map(LoadDecision::Partial),
            Ok(sorted(vec![
                common,
                module,
                object,
                form,
                layout,
                form_module,
                help
            ]))
        );
    }

    #[test]
    fn decide_expands_changed_directory_to_files_with_descriptors() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let configuration = write_source(root, "Configuration.xml", "<xml/>");
        let descriptor = write_source(root, "Catalogs/Items.xml", "<xml/>");
        let directory = root.join("Catalogs/Items");
        let form = write_source(root, "Catalogs/Items/Forms/Form/Ext/Form.xml", "<xml/>");
        write_source(root, "Catalogs/Items/tmp/excluded.xml", "<xml/>");

        let decision = decide(
            &[FileChange {
                path: directory,
                kind: ChangeKind::Modified,
            }],
            root,
        );

        let mut expected = vec![configuration, descriptor, form];
        expected.sort();
        assert_eq!(decision, LoadDecision::Partial(expected));
    }

    /// Список частичной загрузки сборки не несёт файлов `localPath` и `workPath`, даже если
    /// они лежат в развёрнутом каталоге.
    #[test]
    fn a_build_list_leaves_the_runner_directories_out() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let configuration = write_source(root, "Configuration.xml", "<xml/>");
        let descriptor = write_source(root, "Catalogs/Items.xml", "<xml/>");
        let form = write_source(root, "Catalogs/Items/Forms/Form/Ext/Form.xml", "<xml/>");
        write_source(root, "Catalogs/Items/personal/db/1Cv8.1CD", "base");
        write_source(root, "Catalogs/Items/service/state.xml", "<xml/>");
        let local = root.join("Catalogs/Items/personal");
        let work = root.join("Catalogs/Items/service");

        let decision = decide_build(
            &[change(&root.join("Catalogs/Items"), ChangeKind::Modified)],
            root,
            RunnerDirs::new(&work, Some(&local)),
        );

        assert_eq!(decision, sorted(vec![configuration, descriptor, form]));
    }

    #[test]
    fn agent_root_dependencies_include_declared_payloads_only() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        write_source(
            root,
            "Configuration.xml",
            "<MetaDataObject><Configuration><ChildObjects><Catalog>Items</Catalog></ChildObjects></Configuration></MetaDataObject>",
        );
        let descriptor = write_source(root, "Catalogs/Items.xml", "<xml/>");
        let module = write_source(root, "Catalogs/Items/Ext/ObjectModule.bsl", "module");
        let root_module = write_source(root, "Ext/ManagedApplicationModule.bsl", "module");
        write_source(root, "Enums/Items.xml", "<xml/>");
        write_source(root, "v8project.local.yaml", "local-only");

        let mut dependencies = root_dependencies(root).expect("stage dependencies");
        dependencies.sort_unstable();
        dependencies.dedup();
        let mut expected = vec![descriptor.clone(), module, root_module];
        expected.sort_unstable();
        assert_eq!(dependencies, expected);
        std::fs::remove_file(descriptor).expect("remove required descriptor");
        assert!(root_dependencies(root).is_err());
    }

    #[test]
    fn agent_can_stage_the_platform_configuration_and_extension_fixtures() {
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/designer");
        for set in ["configuration", "extension"] {
            let paths = root_dependencies(&fixture.join(set)).expect("all declarations resolve");
            assert!(paths.len() > 3, "{set}");
        }
    }

    #[test]
    fn decide_includes_existing_payloads_of_an_added_object() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let configuration = write_source(root, "Configuration.xml", "<xml/>");
        let descriptor = write_source(root, "Catalogs/Items.xml", "<xml/>");
        let module = write_source(root, "Catalogs/Items/Ext/ObjectModule.bsl", "module");
        let decision = decide(
            &[FileChange {
                path: descriptor.clone(),
                kind: ChangeKind::Added,
            }],
            root,
        );

        let mut expected = vec![configuration, descriptor, module];
        expected.sort();
        assert_eq!(decision, LoadDecision::Partial(expected));
    }

    #[test]
    fn decide_loads_changed_configuration_xml_partially() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let configuration = write_source(root, "Configuration.xml", "<xml/>");

        let decision = decide(
            &[FileChange {
                path: configuration.clone(),
                kind: ChangeKind::Modified,
            }],
            root,
        );

        assert_eq!(decision, LoadDecision::Partial(vec![configuration]));
    }

    #[test]
    fn decide_loads_root_instead_of_deleted_object_files() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let configuration = write_source(
            root,
            "Configuration.xml",
            "<MetaDataObject><Configuration><ChildObjects><Catalog>Other</Catalog></ChildObjects></Configuration></MetaDataObject>",
        );
        let changes = [
            FileChange {
                path: configuration.clone(),
                kind: ChangeKind::Modified,
            },
            FileChange {
                path: root.join("Catalogs/Items.xml"),
                kind: ChangeKind::Deleted,
            },
            FileChange {
                path: root.join("Catalogs/Items/Ext/ObjectModule.bsl"),
                kind: ChangeKind::Deleted,
            },
            FileChange {
                path: root.join("WSReferences/Example.xml"),
                kind: ChangeKind::Deleted,
            },
        ];

        assert_eq!(
            decide(&changes, root),
            LoadDecision::Partial(vec![configuration])
        );
    }

    #[test]
    fn decide_keeps_ambiguous_and_retained_owner_deletions_full() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let configuration = write_source(
            root,
            "Configuration.xml",
            "<MetaDataObject><Configuration><ChildObjects><Catalog>Items</Catalog></ChildObjects></Configuration></MetaDataObject>",
        );
        let root_edit = FileChange {
            path: configuration,
            kind: ChangeKind::Modified,
        };

        for path in [
            "Catalogs/Items.xml",
            "Catalogs/Items/Ext/ObjectModule.bsl",
            "Ext/Help.xml",
        ] {
            let deleted = FileChange {
                path: root.join(path),
                kind: ChangeKind::Deleted,
            };
            assert_eq!(
                decide(&[root_edit.clone(), deleted], root),
                LoadDecision::Full,
                "{path}"
            );
        }
    }

    #[test]
    fn a_retained_name_in_a_different_unicode_form_requires_full_load() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let configuration = write_source(
            root,
            "Configuration.xml",
            "<MetaDataObject><Configuration><ChildObjects><Catalog>Край</Catalog></ChildObjects></Configuration></MetaDataObject>",
        );
        let changes = [
            FileChange {
                path: configuration,
                kind: ChangeKind::Modified,
            },
            FileChange {
                path: root.join("Catalogs/КРАИ\u{306}.xml"),
                kind: ChangeKind::Deleted,
            },
        ];
        assert_eq!(decide(&changes, root), LoadDecision::Full);
    }

    #[test]
    fn a_removed_object_with_changed_descendants_requires_full_load() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let configuration = write_source(
            root,
            "Configuration.xml",
            "<MetaDataObject><Configuration><ChildObjects/></Configuration></MetaDataObject>",
        );
        write_source(root, "Catalogs/Items/Ext/ObjectModule.bsl", "module");
        let changes = [
            FileChange {
                path: configuration,
                kind: ChangeKind::Modified,
            },
            FileChange {
                path: root.join("Catalogs/Items.xml"),
                kind: ChangeKind::Deleted,
            },
            FileChange {
                path: root.join("Catalogs"),
                kind: ChangeKind::Modified,
            },
        ];
        assert_eq!(decide(&changes, root), LoadDecision::Full);
    }

    #[cfg(unix)]
    #[test]
    fn a_removed_object_with_an_escaping_or_dangling_descendant_is_full() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().expect("tempdir");
        let root = temp.path().join("sources");
        let configuration = write_source(
            &root,
            "Configuration.xml",
            "<MetaDataObject><Configuration><ChildObjects/></Configuration></MetaDataObject>",
        );
        std::fs::create_dir(root.join("Catalogs")).expect("catalogs");
        let outside = temp.path().join("outside");
        std::fs::create_dir(&outside).expect("outside");
        let changes = [
            FileChange {
                path: configuration,
                kind: ChangeKind::Modified,
            },
            FileChange {
                path: root.join("Catalogs/Items.xml"),
                kind: ChangeKind::Deleted,
            },
            FileChange {
                path: root.join("Catalogs/Items/Ext/ObjectModule.bsl"),
                kind: ChangeKind::Deleted,
            },
        ];
        for target in [outside, temp.path().join("missing")] {
            let link = root.join("Catalogs/Items");
            symlink(target, &link).expect("symlink");
            assert_eq!(decide(&changes, &root), LoadDecision::Full);
            std::fs::remove_file(link).expect("remove symlink");
        }
    }

    #[cfg(unix)]
    #[test]
    fn unportable_unix_paths_cannot_escape_agent_staging() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        for name in [r"..\escape.xml", ".. /escape.xml", "C:escape.xml"] {
            let file = write_source(root, name, "<xml/>");
            let changes = [FileChange {
                path: file.clone(),
                kind: ChangeKind::Modified,
            }];
            assert_eq!(decide(&changes, root), LoadDecision::Full, "{name}");
            assert!(relative_paths(&[file], root).is_err(), "{name}");
        }
    }

    #[test]
    fn a_path_beginning_with_an_option_prefix_is_not_a_partial_argument() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let file = write_source(root, "--unintended", "<xml/>");
        let changes = [FileChange {
            path: file,
            kind: ChangeKind::Added,
        }];
        assert_eq!(decide(&changes, root), LoadDecision::Full);
    }

    #[test]
    fn decide_loads_more_than_twenty_changed_files_partially() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let changes: Vec<_> = (0..26)
            .map(|index| FileChange {
                path: write_source(
                    root,
                    &format!("CommonModules/Module{index:02}.bsl"),
                    "module",
                ),
                kind: ChangeKind::Modified,
            })
            .collect();
        let expected = changes.iter().map(|change| change.path.clone()).collect();

        assert_eq!(decide(&changes, root), LoadDecision::Partial(expected));
    }

    #[test]
    fn a_traversal_component_is_refused_even_when_it_resolves_inside() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        write_source(root, "Configuration.xml", "<xml/>");
        std::fs::create_dir(root.join("Catalogs")).expect("mkdir");

        let decision = decide(
            &[FileChange {
                path: root.join("Catalogs/../Configuration.xml"),
                kind: ChangeKind::Modified,
            }],
            root,
        );
        assert_eq!(decision, LoadDecision::Full);
    }

    #[cfg(unix)]
    #[test]
    fn traversal_or_symlink_escape_forces_full() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().expect("tempdir");
        let root = temp.path().join("src");
        let outside_dir = temp.path().join("outside");
        let link_dir = root.join("Catalogs.Items");
        let escaped = outside_dir.join("ObjectModule.bsl");

        std::fs::create_dir_all(&outside_dir).expect("outside");
        std::fs::create_dir_all(&root).expect("root");
        std::fs::write(&escaped, "module").expect("write escaped");
        symlink(&outside_dir, &link_dir).expect("create symlink");

        let decision = decide(
            &[FileChange {
                path: link_dir.join("ObjectModule.bsl"),
                kind: ChangeKind::Modified,
            }],
            &root,
        );

        assert_eq!(decision, LoadDecision::Full);
    }
}
