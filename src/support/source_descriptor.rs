use std::path::{Path, PathBuf};

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::support::edt_project;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceDescriptorPurpose {
    Configuration,
    Extension,
    ExternalDataProcessors,
    ExternalReports,
}

impl SourceDescriptorPurpose {
    pub const fn is_external(self) -> bool {
        matches!(self, Self::ExternalDataProcessors | Self::ExternalReports)
    }

    pub const fn external_root_tag(self) -> Option<&'static str> {
        match self {
            Self::ExternalDataProcessors => Some("ExternalDataProcessor"),
            Self::ExternalReports => Some("ExternalReport"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedExternalDescriptor {
    pub logical_name: String,
    pub purpose: SourceDescriptorPurpose,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceDescriptorParseError {
    Xml(String),
    UnexpectedEof,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalDescriptorParseError {
    Xml(String),
    DecodeLogicalName(String),
    MissingRootElement,
    UnsupportedRootElement(String),
    MissingLogicalName,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceSetRootScanError {
    Runtime(String),
    Validation(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesignerExternalDescriptorEntry {
    pub path: PathBuf,
    pub purpose: Option<SourceDescriptorPurpose>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdtExternalProjectEntry {
    pub path: PathBuf,
    pub purpose: Option<SourceDescriptorPurpose>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum XmlDescriptorKind {
    Configuration,
    ExternalDataProcessor,
    ExternalReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct XmlDescriptorScan {
    kind: Option<XmlDescriptorKind>,
    effective_tag: Option<String>,
    has_configuration_extension_purpose: bool,
    has_object_belonging: bool,
}

pub fn classify_source_descriptor(
    content: &str,
) -> Result<Option<SourceDescriptorPurpose>, SourceDescriptorParseError> {
    let scan = scan_xml_descriptor(content)?;
    Ok(match scan.kind {
        Some(XmlDescriptorKind::Configuration) => {
            if scan.has_configuration_extension_purpose || scan.has_object_belonging {
                Some(SourceDescriptorPurpose::Extension)
            } else {
                Some(SourceDescriptorPurpose::Configuration)
            }
        }
        Some(XmlDescriptorKind::ExternalDataProcessor) => {
            Some(SourceDescriptorPurpose::ExternalDataProcessors)
        }
        Some(XmlDescriptorKind::ExternalReport) => Some(SourceDescriptorPurpose::ExternalReports),
        None => None,
    })
}

pub fn classify_external_source_descriptor(
    content: &str,
) -> Result<Option<SourceDescriptorPurpose>, SourceDescriptorParseError> {
    match classify_source_descriptor(content)? {
        Some(purpose) if purpose.is_external() => Ok(Some(purpose)),
        _ => Ok(None),
    }
}

pub fn parse_external_descriptor(
    content: &str,
) -> Result<ParsedExternalDescriptor, ExternalDescriptorParseError> {
    let scan = scan_xml_descriptor(content).map_err(|error| match error {
        SourceDescriptorParseError::Xml(error) => ExternalDescriptorParseError::Xml(error),
        SourceDescriptorParseError::UnexpectedEof => {
            ExternalDescriptorParseError::Xml("unexpected EOF".to_owned())
        }
    })?;

    let purpose = match scan.kind {
        Some(XmlDescriptorKind::ExternalDataProcessor) => {
            SourceDescriptorPurpose::ExternalDataProcessors
        }
        Some(XmlDescriptorKind::ExternalReport) => SourceDescriptorPurpose::ExternalReports,
        Some(XmlDescriptorKind::Configuration) => {
            return Err(ExternalDescriptorParseError::UnsupportedRootElement(
                scan.effective_tag
                    .unwrap_or_else(|| "Configuration".to_owned()),
            ))
        }
        None => match scan.effective_tag {
            Some(root) => return Err(ExternalDescriptorParseError::UnsupportedRootElement(root)),
            None => return Err(ExternalDescriptorParseError::MissingRootElement),
        },
    };

    let logical_name = extract_logical_name(content)?
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or(ExternalDescriptorParseError::MissingLogicalName)?;

    Ok(ParsedExternalDescriptor {
        logical_name,
        purpose,
    })
}

pub fn extract_configuration_logical_name(
    content: &str,
) -> Result<Option<String>, SourceDescriptorParseError> {
    let scan = scan_xml_descriptor(content)?;
    if scan.kind != Some(XmlDescriptorKind::Configuration) {
        return Ok(None);
    }

    extract_configuration_name(content)
}

pub fn scan_designer_external_root(
    dir: &Path,
) -> Result<Vec<DesignerExternalDescriptorEntry>, SourceSetRootScanError> {
    let entries = std::fs::read_dir(dir).map_err(|error| {
        SourceSetRootScanError::Runtime(format!(
            "failed to read source directory '{}': {error}",
            dir.display()
        ))
    })?;

    let mut descriptors = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            SourceSetRootScanError::Runtime(format!(
                "failed to read source directory entry '{}': {error}",
                dir.display()
            ))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            SourceSetRootScanError::Runtime(format!(
                "failed to inspect source directory entry '{}': {error}",
                dir.display()
            ))
        })?;
        let path = entry.path();
        if file_type.is_symlink()
            || !file_type.is_file()
            || path
                .extension()
                .and_then(|value| value.to_str())
                .is_none_or(|value| !value.eq_ignore_ascii_case("xml"))
        {
            continue;
        }
        let content = std::fs::read_to_string(&path).map_err(|error| {
            SourceSetRootScanError::Runtime(format!(
                "failed to read source descriptor '{}': {error}",
                path.display()
            ))
        })?;
        let purpose =
            classify_external_source_descriptor(&content).map_err(|error| match error {
                SourceDescriptorParseError::Xml(error) => {
                    SourceSetRootScanError::Validation(format!(
                        "failed to parse source descriptor '{}': {error}",
                        path.display()
                    ))
                }
                SourceDescriptorParseError::UnexpectedEof => {
                    SourceSetRootScanError::Validation(format!(
                        "failed to parse source descriptor '{}': unexpected EOF",
                        path.display()
                    ))
                }
            })?;
        descriptors.push(DesignerExternalDescriptorEntry { path, purpose });
    }

    Ok(descriptors)
}

pub fn scan_edt_external_root(
    dir: &Path,
) -> Result<Vec<EdtExternalProjectEntry>, SourceSetRootScanError> {
    let entries = std::fs::read_dir(dir).map_err(|error| {
        SourceSetRootScanError::Runtime(format!(
            "failed to read source directory '{}': {error}",
            dir.display()
        ))
    })?;

    let mut projects = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            SourceSetRootScanError::Runtime(format!(
                "failed to read source directory entry '{}': {error}",
                dir.display()
            ))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            SourceSetRootScanError::Runtime(format!(
                "failed to inspect source directory entry '{}': {error}",
                dir.display()
            ))
        })?;
        let path = entry.path();
        if file_type.is_symlink() || !file_type.is_dir() || !path.join(".project").is_file() {
            continue;
        }
        let purpose = detect_edt_external_project_purpose(&path)?;
        projects.push(EdtExternalProjectEntry { path, purpose });
    }

    Ok(projects)
}

fn scan_xml_descriptor(content: &str) -> Result<XmlDescriptorScan, SourceDescriptorParseError> {
    let mut reader = Reader::from_str(content);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut root_tag = None::<String>;
    let mut first_child_tag = None::<String>;
    let mut depth = 0usize;
    let mut scan = XmlDescriptorScan::default();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(event)) => {
                let tag = xml_local_name(event.name().as_ref());
                if root_tag.is_none() {
                    root_tag = Some(tag.clone());
                    depth = 1;
                } else {
                    if depth == 1 && first_child_tag.is_none() {
                        first_child_tag = Some(tag.clone());
                    }
                    depth += 1;
                }
                if tag == "ConfigurationExtensionPurpose" {
                    scan.has_configuration_extension_purpose = true;
                } else if tag == "ObjectBelonging" {
                    scan.has_object_belonging = true;
                }
            }
            Ok(Event::Empty(event)) => {
                let tag = xml_local_name(event.name().as_ref());
                if root_tag.is_none() {
                    root_tag = Some(tag.clone());
                    break;
                }
                if depth == 1 && first_child_tag.is_none() {
                    first_child_tag = Some(tag.clone());
                }
                if tag == "ConfigurationExtensionPurpose" {
                    scan.has_configuration_extension_purpose = true;
                } else if tag == "ObjectBelonging" {
                    scan.has_object_belonging = true;
                }
            }
            Ok(Event::End(_)) => {
                depth = depth.saturating_sub(1);
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(SourceDescriptorParseError::Xml(error.to_string()));
            }
            _ => {}
        }
        buf.clear();
    }

    if depth > 0 {
        return Err(SourceDescriptorParseError::UnexpectedEof);
    }

    let effective_tag = match root_tag.as_deref() {
        Some("MetaDataObject") => first_child_tag.as_deref(),
        other => other,
    };
    scan.effective_tag = effective_tag.map(ToOwned::to_owned);
    scan.kind = match effective_tag {
        Some("Configuration") => Some(XmlDescriptorKind::Configuration),
        Some("ExternalDataProcessor") => Some(XmlDescriptorKind::ExternalDataProcessor),
        Some("ExternalReport") => Some(XmlDescriptorKind::ExternalReport),
        _ => None,
    };

    Ok(scan)
}

fn extract_logical_name(content: &str) -> Result<Option<String>, ExternalDescriptorParseError> {
    let mut reader = Reader::from_str(content);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut seen_properties = false;
    let mut seen_name = false;
    let mut logical_name = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(event)) => {
                let tag = xml_local_name(event.name().as_ref());
                if tag == "Properties" {
                    seen_properties = true;
                } else if seen_properties && tag == "Name" {
                    seen_name = true;
                }
            }
            Ok(Event::Text(text)) if seen_name && logical_name.is_none() => {
                logical_name = Some(
                    text.unescape()
                        .map_err(|error| {
                            ExternalDescriptorParseError::DecodeLogicalName(error.to_string())
                        })?
                        .into_owned(),
                );
            }
            Ok(Event::End(event)) => {
                let tag = xml_local_name(event.name().as_ref());
                if tag == "Name" {
                    seen_name = false;
                } else if tag == "Properties" {
                    seen_properties = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(ExternalDescriptorParseError::Xml(error.to_string()));
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(logical_name)
}

fn extract_configuration_name(content: &str) -> Result<Option<String>, SourceDescriptorParseError> {
    let mut reader = Reader::from_str(content);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut stack = Vec::<String>::new();
    let mut logical_name = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(event)) => {
                stack.push(xml_local_name(event.name().as_ref()));
            }
            Ok(Event::Empty(event)) => {
                stack.push(xml_local_name(event.name().as_ref()));
                stack.pop();
            }
            Ok(Event::Text(text))
                if logical_name.is_none() && is_configuration_name_path(&stack) =>
            {
                logical_name = Some(
                    text.unescape()
                        .map_err(|error| SourceDescriptorParseError::Xml(error.to_string()))?
                        .into_owned(),
                );
            }
            Ok(Event::End(_)) => {
                stack.pop();
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(SourceDescriptorParseError::Xml(error.to_string()));
            }
            _ => {}
        }
        buf.clear();
    }

    if !stack.is_empty() {
        return Err(SourceDescriptorParseError::UnexpectedEof);
    }

    Ok(logical_name)
}

fn is_configuration_name_path(stack: &[String]) -> bool {
    matches!(
        stack,
        [configuration, name] if configuration == "Configuration" && name == "name"
    ) || matches!(
        stack,
        [configuration, properties, name]
            if configuration == "Configuration" && properties == "Properties" && name == "Name"
    ) || matches!(
        stack,
        [metadata, configuration, properties, name]
            if metadata == "MetaDataObject"
                && configuration == "Configuration"
                && properties == "Properties"
                && name == "Name"
    )
}

fn detect_edt_external_project_purpose(
    project_dir: &Path,
) -> Result<Option<SourceDescriptorPurpose>, SourceSetRootScanError> {
    if !edt_project::has_native_external_project_layout(project_dir)
        .map_err(|error| SourceSetRootScanError::Validation(error.to_string()))?
    {
        return Ok(None);
    }

    let descriptor_path = edt_project::external_root_descriptor_path(project_dir);
    let content = std::fs::read_to_string(&descriptor_path).map_err(|error| {
        SourceSetRootScanError::Runtime(format!(
            "failed to read source descriptor '{}': {error}",
            descriptor_path.display()
        ))
    })?;
    classify_external_source_descriptor(&content).map_err(|error| match error {
        SourceDescriptorParseError::Xml(error) => SourceSetRootScanError::Validation(format!(
            "failed to parse source descriptor '{}': {error}",
            descriptor_path.display()
        )),
        SourceDescriptorParseError::UnexpectedEof => SourceSetRootScanError::Validation(format!(
            "failed to parse source descriptor '{}': unexpected EOF",
            descriptor_path.display()
        )),
    })
}

fn xml_local_name(name: &[u8]) -> String {
    let raw = String::from_utf8_lossy(name);
    raw.rsplit(':').next().unwrap_or(raw.as_ref()).to_owned()
}

/// Types and names directly declared by a Designer configuration's ChildObjects.
pub(crate) fn designer_child_objects(content: &str) -> Option<Vec<(String, String)>> {
    let mut reader = Reader::from_str(content.trim_start_matches('\u{feff}'));
    reader.config_mut().trim_text(true);
    let mut stack = Vec::new();
    let mut names = None;
    let mut name = String::new();
    let mut seen_root = false;
    let mut seen_configuration = false;

    loop {
        match reader.read_event().ok()? {
            Event::Start(event) => {
                let tag = xml_local_name(event.name().as_ref());
                match stack.len() {
                    0 if !seen_root && tag == "MetaDataObject" => seen_root = true,
                    1 if !seen_configuration && tag == "Configuration" => seen_configuration = true,
                    0 | 1 => return None,
                    2 if tag == "ChildObjects" => {
                        if names.replace(Vec::new()).is_some() {
                            return None;
                        }
                    }
                    3 if stack[2] == "ChildObjects" => name.clear(),
                    4.. if stack[2] == "ChildObjects" => return None,
                    _ => {}
                }
                stack.push(tag);
            }
            Event::Empty(event)
                if stack.len() == 2 && event.local_name().as_ref() == b"ChildObjects" =>
            {
                if names.replace(Vec::new()).is_some() {
                    return None;
                }
            }
            Event::Text(text) if stack.len() == 4 && stack[2] == "ChildObjects" => {
                name.push_str(&text.unescape().ok()?);
            }
            Event::End(_) => {
                if stack.len() == 4 && stack[2] == "ChildObjects" {
                    if name.is_empty() {
                        return None;
                    }
                    names
                        .as_mut()?
                        .push((stack[3].clone(), std::mem::take(&mut name)));
                }
                stack.pop()?;
            }
            Event::Eof if stack.is_empty() => return names,
            Event::Eof => return None,
            Event::Empty(_) | Event::CData(_) | Event::Text(_)
                if stack.len() >= 3 && stack[2] == "ChildObjects" =>
            {
                return None;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        classify_source_descriptor, extract_configuration_logical_name, parse_external_descriptor,
        scan_designer_external_root, SourceDescriptorPurpose,
    };
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn classify_configuration_extension_marker_as_extension() {
        let xml = "<Configuration><ObjectBelonging>Adopted</ObjectBelonging></Configuration>";

        assert_eq!(
            classify_source_descriptor(xml).expect("classify"),
            Some(SourceDescriptorPurpose::Extension)
        );
    }

    #[test]
    fn extracts_configuration_logical_name_from_designer_descriptor() {
        let xml = "<MetaDataObject><Configuration><Properties><Name>client_mcp</Name><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose></Properties></Configuration></MetaDataObject>";

        assert_eq!(
            extract_configuration_logical_name(xml).expect("name"),
            Some("client_mcp".to_owned())
        );
    }

    #[test]
    fn extracts_configuration_logical_name_from_edt_descriptor() {
        let xml = r#"<mdclass:Configuration xmlns:mdclass="http://g5.1c.ru/v8/dt/metadata/mdclass"><name>test_client</name><languages><name>Русский</name></languages></mdclass:Configuration>"#;

        assert_eq!(
            extract_configuration_logical_name(xml).expect("name"),
            Some("test_client".to_owned())
        );
    }

    #[test]
    fn parse_external_descriptor_accepts_metadataobject_wrapper() {
        let xml = "<MetaDataObject><ExternalDataProcessor><Properties><Name>Foo</Name></Properties></ExternalDataProcessor></MetaDataObject>";
        let parsed = parse_external_descriptor(xml).expect("parse");

        assert_eq!(parsed.logical_name, "Foo");
        assert_eq!(
            parsed.purpose,
            SourceDescriptorPurpose::ExternalDataProcessors
        );
    }

    #[test]
    fn scan_designer_external_root_accepts_uppercase_xml_extension() {
        let dir = tempdir().expect("tempdir");
        fs::write(
            dir.path().join("Report.XML"),
            "<ExternalReport><Properties><Name>Report</Name></Properties></ExternalReport>",
        )
        .expect("descriptor");

        let entries = scan_designer_external_root(dir.path()).expect("scan");

        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].purpose,
            Some(SourceDescriptorPurpose::ExternalReports)
        );
    }
}
