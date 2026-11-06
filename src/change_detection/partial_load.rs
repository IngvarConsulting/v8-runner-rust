use std::collections::HashSet;
use std::io::{self, ErrorKind};
use std::path::{Component, Path, PathBuf};

use crate::change_detection::analyzer::{ChangeKind, FileChange};
use crate::change_detection::scanner;
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
    match expand_files(changes, source_root) {
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
        expand_optional_directory(&mut paths, &payload, source_root, &root_real)
            .ok_or_else(|| invalid(&payload))?;
    }
    let root_payload = source_root.join("Ext");
    expand_optional_directory(&mut paths, &root_payload, source_root, &root_real)
        .ok_or_else(|| invalid(&root_payload))?;
    Ok(paths)
}

fn expand_files(changes: &[FileChange], source_root: &Path) -> Option<Vec<PathBuf>> {
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
            expand_directory(&mut paths, &path, source_root, &root_real)?;
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
                expand_optional_directory(&mut paths, &object_dir, source_root, &root_real)?;
            }
            push_with_descriptors(&mut paths, path, source_root, &root_real)?;
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
) -> Option<()> {
    match std::fs::symlink_metadata(directory) {
        Ok(_) => expand_directory(paths, directory, source_root, root_real),
        Err(error) if error.kind() == ErrorKind::NotFound => Some(()),
        Err(_) => None,
    }
}

fn expand_directory(
    paths: &mut Vec<PathBuf>,
    directory: &Path,
    source_root: &Path,
    root_real: &Path,
) -> Option<()> {
    // The scanner does not select symlink directories within a source set.
    if directory != source_root && !std::fs::symlink_metadata(directory).ok()?.is_dir() {
        return None;
    }
    for entry in scanner::source_entries(directory) {
        let entry = entry.ok()?;
        let relative = safe_relative_path(entry.path(), source_root, root_real)?;
        if scanner::is_source_file(&entry) {
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

fn push_with_descriptors(
    paths: &mut Vec<PathBuf>,
    path: PathBuf,
    source_root: &Path,
    root_real: &Path,
) -> Option<()> {
    // Both direct changes and expanded entries have already passed the path guard.
    if !path.is_file() {
        return None;
    }
    if is_bsl(&path) {
        if let Some(xml) = sibling_xml(&path) {
            push_file_if_safe_if_exists(paths, &xml, source_root, root_real)?;
        }
        for descriptor in ancestor_xml_descriptors(path.parent(), source_root) {
            push_file_if_safe_if_exists(paths, &descriptor, source_root, root_real)?;
        }
    }
    paths.push(path);
    Some(())
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

    use super::{decide, relative_paths, root_dependencies, write_list_file, LoadDecision};
    use crate::change_detection::analyzer::{ChangeKind, FileChange};

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

    #[test]
    fn decide_expands_nested_bsl_to_ancestor_xml_files_without_directories() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let module = root.join("Catalogs/Items/Forms/ItemForm/Ext/Form/Module.bsl");
        let form_xml = root.join("Catalogs/Items/Forms/ItemForm.xml");
        let form_ext_xml = root.join("Catalogs/Items/Forms/ItemForm/Ext/Form.xml");
        let object_xml = root.join("Catalogs/Items.xml");

        std::fs::create_dir_all(module.parent().expect("module parent")).expect("module dir");
        std::fs::write(&module, "module").expect("write module");
        std::fs::write(&form_xml, "<form />").expect("write form xml");
        std::fs::write(&form_ext_xml, "<form ext />").expect("write form ext xml");
        std::fs::write(&object_xml, "<object />").expect("write object xml");

        let decision = decide(
            &[FileChange {
                path: module.clone(),
                kind: ChangeKind::Modified,
            }],
            root,
        );

        let LoadDecision::Partial(paths) = decision else {
            panic!("expected partial decision");
        };

        assert_eq!(paths, vec![module, form_ext_xml, form_xml, object_xml]);
        assert!(paths.iter().all(|path| path.is_file()));
    }

    fn write_source(root: &std::path::Path, name: &str, content: &str) -> PathBuf {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, content).expect("write source");
        path
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
