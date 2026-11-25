use std::io;

use quick_xml::events::BytesStart;
use quick_xml::events::Event;
use quick_xml::name::ResolveResult;
use quick_xml::{NsReader, Writer};

use crate::domain::launch::DebuggerUrl;

const VRD: &[u8] = b"http://v8.1c.ru/8.2/virtual-resource-system";
const DEBUG_KEYS: [&[u8]; 3] = [b"enable", b"protocol", b"url"];

/// Whether a managed VRD describes the requested published path.
pub(crate) fn publication_alias_matches(contents: &[u8], wsdir: &str) -> io::Result<bool> {
    let publication = inspect_publication(contents, None)?;
    let requested = format!("/{wsdir}");
    Ok(publication
        .alias
        .is_some_and(|alias| alias.strip_suffix('/').unwrap_or(&alias) == requested))
}

/// `None` means the publication already has this route. Return a byte-preserving edit
/// otherwise; only the publication's direct `debug` element belongs to this operation.
pub(crate) fn publication_debug_edit(
    contents: &[u8],
    debugger: &DebuggerUrl,
) -> io::Result<Option<Vec<u8>>> {
    Ok(inspect_publication(contents, Some(&debugger.to_string()))?.edit)
}

struct Publication {
    alias: Option<String>,
    edit: Option<Vec<u8>>,
}

fn inspect_publication(contents: &[u8], debugger: Option<&str>) -> io::Result<Publication> {
    let bom_len = usize::from(contents.starts_with(b"\xef\xbb\xbf")) * 3;
    let mut reader = NsReader::from_reader(contents);
    let mut depth = 0usize;
    let mut debug_name = None;
    let mut alias = None;
    let mut edit = None;
    let mut found_debug = false;
    let mut closed = false;

    loop {
        let start = reader.buffer_position() as usize + bom_len;
        let (namespace, event) = reader.read_resolved_event().map_err(io::Error::other)?;
        let vrd = matches!(namespace, ResolveResult::Bound(namespace) if namespace.as_ref() == VRD);
        let end = reader.buffer_position() as usize + bom_len;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) if depth == 0 => {
                if !vrd || element.local_name().as_ref() != b"point" || debug_name.is_some() {
                    return Err(io::Error::other("expected one VRD publication point"));
                }
                for attribute in element.attributes() {
                    let attribute = attribute.map_err(io::Error::other)?;
                    if attribute.key.as_ref() == b"base" {
                        alias = Some(
                            attribute
                                .unescape_value()
                                .map_err(io::Error::other)?
                                .into_owned(),
                        );
                    }
                }
                let name = element.name();
                let qualified = match name.prefix() {
                    Some(prefix) => format!(
                        "{}:debug",
                        std::str::from_utf8(prefix.as_ref()).map_err(io::Error::other)?
                    ),
                    None => "debug".to_owned(),
                };
                if matches!(event, Event::Empty(_)) {
                    closed = true;
                    if let Some(debugger) = debugger {
                        let mut writer = Writer::new(Vec::new());
                        writer
                            .write_event(Event::Start(element.borrow()))
                            .map_err(io::Error::other)?;
                        writer
                            .write_event(debug_element(&qualified, debugger))
                            .map_err(io::Error::other)?;
                        writer
                            .write_event(Event::End(element.to_end()))
                            .map_err(io::Error::other)?;
                        edit = Some((start, end, writer.into_inner()));
                    }
                } else {
                    depth = 1;
                }
                debug_name = Some(qualified);
            }
            Event::Start(ref element) | Event::Empty(ref element) => {
                if depth == 1 && vrd && element.local_name().as_ref() == b"debug" {
                    if found_debug {
                        return Err(io::Error::other(
                            "the publication has multiple debug elements",
                        ));
                    }
                    found_debug = true;
                    let new = debugger
                        .map(|debugger| edit_debug(element, debugger))
                        .transpose()?
                        .flatten();
                    if let Some(new) = new {
                        let mut writer = Writer::new(Vec::new());
                        if matches!(event, Event::Start(_)) {
                            writer
                                .write_event(Event::Start(new))
                                .map_err(io::Error::other)?;
                        } else {
                            writer
                                .write_event(Event::Empty(new))
                                .map_err(io::Error::other)?;
                        }
                        edit = Some((start, end, writer.into_inner()));
                    }
                }
                if matches!(event, Event::Start(_)) {
                    depth += 1;
                }
            }
            Event::End(_) => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| io::Error::other("unmatched VRD end element"))?;
                if depth == 0 {
                    closed = true;
                    if let Some(debugger) = debugger.filter(|_| !found_debug) {
                        let mut writer = Writer::new(Vec::new());
                        writer
                            .write_event(debug_element(
                                debug_name.as_deref().ok_or_else(|| {
                                    io::Error::other("missing VRD publication point")
                                })?,
                                debugger,
                            ))
                            .map_err(io::Error::other)?;
                        edit = Some((start, start, writer.into_inner()));
                    }
                }
            }
            Event::Text(text) if depth == 0 && !text.iter().all(u8::is_ascii_whitespace) => {
                return Err(io::Error::other("text outside the VRD publication point"));
            }
            Event::Eof if closed => break,
            Event::Eof => return Err(io::Error::other("incomplete VRD publication point")),
            _ => {}
        }
    }

    Ok(Publication {
        alias,
        edit: edit.map(|(start, end, replacement)| {
            [&contents[..start], &replacement, &contents[end..]].concat()
        }),
    })
}

fn debug_element<'a>(name: &'a str, debugger: &str) -> Event<'a> {
    Event::Empty(BytesStart::new(name).with_attributes([
        ("enable", "true"),
        ("protocol", "http"),
        ("url", debugger),
    ]))
}

fn edit_debug<'a>(element: &BytesStart<'a>, debugger: &str) -> io::Result<Option<BytesStart<'a>>> {
    let values = ["true", "http", debugger];
    let mut current = [false; 3];
    let mut updated = element.clone();
    updated.clear_attributes();
    for attribute in element.attributes() {
        let attribute = attribute.map_err(io::Error::other)?;
        if let Some(index) = DEBUG_KEYS
            .iter()
            .position(|key| *key == attribute.key.as_ref())
        {
            current[index] = attribute.unescape_value().map_err(io::Error::other)? == values[index];
        } else {
            let value = attribute.unescape_value().map_err(io::Error::other)?;
            let key = std::str::from_utf8(attribute.key.as_ref()).map_err(io::Error::other)?;
            updated.push_attribute((key, value.as_ref()));
        }
    }
    if current.into_iter().all(|matches| matches) {
        return Ok(None);
    }
    updated.extend_attributes([("enable", "true"), ("protocol", "http"), ("url", debugger)]);
    Ok(Some(updated))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_publication_alias_is_the_vrd_root_base() {
        let vrd = br#"<?xml version="1.0"?>
            <v:point xmlns:v="http://v8.1c.ru/8.2/virtual-resource-system" base="/demo/">
              <v:ws base="/renamed"/>
            </v:point>"#;
        assert!(publication_alias_matches(vrd, "demo").unwrap());
        assert!(!publication_alias_matches(vrd, "renamed").unwrap());
        assert!(publication_alias_matches(br#"<point base="/demo"/>"#, "demo").is_err());
        assert!(publication_alias_matches(&vrd[..vrd.len() - 10], "demo").is_err());
        assert!(
            publication_alias_matches(&[vrd.as_slice(), b"<other/>"].concat(), "demo").is_err()
        );
    }

    #[test]
    fn an_edit_keeps_the_bom_and_other_vrd_settings() {
        let debugger = DebuggerUrl::parse("http://localhost:1561").unwrap();
        let vrd = concat!(
            "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
            "<v:point xmlns:v=\"http://v8.1c.ru/8.2/virtual-resource-system\">\r\n",
            "  <v:ws keep=\"&lt;hidden&gt;\"/>\r\n",
            "  <v:debug enable='false' other='A \"quoted\" &amp; value'/>\r\n",
            "</v:point>\r\n"
        );
        let updated = publication_debug_edit(vrd.as_bytes(), &debugger)
            .unwrap()
            .unwrap();
        let updated = std::str::from_utf8(&updated).unwrap();
        let expected = vrd.replace(
            "enable='false' other='A \"quoted\" &amp; value'",
            "other=\"A &quot;quoted&quot; &amp; value\" enable=\"true\" protocol=\"http\" url=\"http://localhost:1561\"",
        );
        assert_eq!(updated, expected);
        assert!(publication_debug_edit(updated.as_bytes(), &debugger)
            .unwrap()
            .is_none());
    }
}
