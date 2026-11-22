use std::io;

use quick_xml::events::BytesStart;
use quick_xml::events::Event;
use quick_xml::name::ResolveResult;
use quick_xml::{NsReader, Writer};

use crate::domain::launch::DebuggerUrl;

const VRD: &[u8] = b"http://v8.1c.ru/8.2/virtual-resource-system";
const DEBUG_KEYS: [&[u8]; 3] = [b"enable", b"protocol", b"url"];

/// `None` means the publication already has this route. Return a byte-preserving edit
/// otherwise; only the publication's direct `debug` element belongs to this operation.
pub(crate) fn publication_debug_edit(
    contents: &[u8],
    debugger: &DebuggerUrl,
) -> io::Result<Option<Vec<u8>>> {
    let bom_len = usize::from(contents.starts_with(b"\xef\xbb\xbf")) * 3;
    let mut reader = NsReader::from_reader(contents);
    let mut depth = 0usize;
    let mut debug_name = None;
    let mut edit = None;
    let mut found_debug = false;
    let mut closed = false;
    let debugger = debugger.to_string();

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
                let name = element.name();
                let qualified = match name.prefix() {
                    Some(prefix) => format!(
                        "{}:debug",
                        std::str::from_utf8(prefix.as_ref()).map_err(io::Error::other)?
                    ),
                    None => "debug".to_owned(),
                };
                if matches!(event, Event::Empty(_)) {
                    let mut writer = Writer::new(Vec::new());
                    writer
                        .write_event(Event::Start(element.borrow()))
                        .map_err(io::Error::other)?;
                    writer
                        .write_event(debug_element(&qualified, &debugger))
                        .map_err(io::Error::other)?;
                    writer
                        .write_event(Event::End(element.to_end()))
                        .map_err(io::Error::other)?;
                    edit = Some((start, end, writer.into_inner()));
                    closed = true;
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
                    if let Some(new) = edit_debug(element, &debugger)? {
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
                    if !found_debug {
                        let mut writer = Writer::new(Vec::new());
                        writer
                            .write_event(debug_element(
                                debug_name.as_deref().ok_or_else(|| {
                                    io::Error::other("missing VRD publication point")
                                })?,
                                &debugger,
                            ))
                            .map_err(io::Error::other)?;
                        edit = Some((start, start, writer.into_inner()));
                    }
                }
            }
            Event::Eof if closed => break,
            Event::Eof => return Err(io::Error::other("incomplete VRD publication point")),
            _ => {}
        }
    }

    Ok(edit.map(|(start, end, replacement)| {
        [&contents[..start], &replacement, &contents[end..]].concat()
    }))
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
