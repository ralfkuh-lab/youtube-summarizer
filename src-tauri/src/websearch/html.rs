//! Grobe HTML-zu-Text-Umwandlung ohne zusaetzliche Crate. Ein Durchlauf ueber
//! den Text (linear), keine wiederholten Scans.

/// Blockelemente: ihr oeffnendes/schliessendes Tag erzeugt einen Zeilenumbruch.
const BLOCK_ELEMENTS: &[&str] = &[
    "p",
    "div",
    "br",
    "li",
    "tr",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "blockquote",
    "pre",
    "section",
    "article",
];

/// Elemente, deren Inhalt komplett verworfen wird (nur mit Abschluss-Tag).
const SKIPPED_ELEMENTS: &[&str] = &["script", "style", "noscript"];

/// `script`/`style`/`noscript` samt Inhalt entfernen, Kommentare entfernen,
/// Tags strippen, Entities dekodieren, Whitespace normalisieren.
pub fn html_to_text(html: &str) -> String {
    let lowered = html.to_ascii_lowercase();
    let bytes = html.as_bytes();
    let mut out = String::with_capacity(html.len());
    let mut index = 0usize;

    while index < bytes.len() {
        // Text bis zum naechsten '<' uebernehmen.
        if bytes[index] != b'<' {
            let next = html[index..].find('<').map(|offset| index + offset);
            match next {
                Some(next) => {
                    out.push_str(&html[index..next]);
                    index = next;
                }
                None => {
                    out.push_str(&html[index..]);
                    break;
                }
            }
            continue;
        }

        // Ein '<' ohne Tag-Anfang (Buchstabe, '/', '!', '?') bleibt Text:
        // "a < b und c > d".
        let is_tag_start = matches!(
            bytes.get(index + 1),
            Some(byte) if byte.is_ascii_alphabetic() || *byte == b'/' || *byte == b'!' || *byte == b'?'
        );
        if !is_tag_start {
            out.push('<');
            index += 1;
            continue;
        }

        // Kommentare als Einheit entfernen (sie koennen '>' enthalten).
        if lowered[index..].starts_with("<!--") {
            index = match lowered[index + 4..].find("-->") {
                Some(offset) => index + 4 + offset + 3,
                None => html.len(),
            };
            continue;
        }

        let Some(tag_end_offset) = html[index..].find('>') else {
            // Kein Tag-Ende: das '<' gehoert zum Text.
            out.push('<');
            index += 1;
            continue;
        };
        let tag_end = index + tag_end_offset;
        let name = tag_name(&lowered[index..=tag_end]);
        if BLOCK_ELEMENTS.contains(&name) {
            out.push('\n');
        }

        if SKIPPED_ELEMENTS.contains(&name) {
            let closing = format!("</{name}");
            match lowered[tag_end + 1..].find(&closing) {
                Some(offset) => {
                    let after_close = tag_end + 1 + offset + closing.len();
                    index = lowered[after_close..]
                        .find('>')
                        .map(|position| after_close + position + 1)
                        .unwrap_or(html.len());
                }
                // Unabgeschlossen: nur das oeffnende Tag ueberspringen, der Rest
                // bleibt als Text erhalten.
                None => index = tag_end + 1,
            }
            continue;
        }

        index = tag_end + 1;
    }

    normalize_text(&decode_entities(&out))
}

/// Tag-Name ohne '<', '</' und Attribute, kleingeschrieben (Eingabe ist bereits
/// kleingeschrieben).
fn tag_name(tag: &str) -> &str {
    let rest = tag
        .strip_prefix("</")
        .or_else(|| tag.strip_prefix('<'))
        .unwrap_or(tag);
    let end = rest
        .find(|ch: char| !ch.is_ascii_alphanumeric())
        .unwrap_or(rest.len());
    &rest[..end]
}

/// Whitespace je Zeile zu einem Leerzeichen, Zeilen trimmen, hoechstens eine
/// Leerzeile in Folge, keine fuehrenden/abschliessenden Leerzeilen.
fn normalize_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank_pending = false;
    for raw_line in text.split('\n') {
        let line = raw_line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            if !out.is_empty() {
                blank_pending = true;
            }
            continue;
        }
        if blank_pending {
            out.push('\n');
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&line);
        blank_pending = false;
    }
    out
}

fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(position) = rest.find('&') {
        out.push_str(&rest[..position]);
        rest = &rest[position..];
        let entity_end = rest
            .char_indices()
            .take(12)
            .find(|(_, ch)| *ch == ';')
            .map(|(offset, _)| offset);
        match entity_end.and_then(|end| decode_entity(&rest[1..end]).map(|value| (end, value))) {
            Some((end, value)) => {
                out.push_str(&value);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_entity(entity: &str) -> Option<String> {
    if let Some(number) = entity.strip_prefix('#') {
        let hex = number
            .strip_prefix('x')
            .or_else(|| number.strip_prefix('X'));
        let code = match hex {
            Some(value) => u32::from_str_radix(value, 16).ok()?,
            None => number.parse::<u32>().ok()?,
        };
        return char::from_u32(code).map(String::from);
    }
    let value = match entity {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" | "39" => "'",
        "nbsp" => " ",
        "hellip" => "…",
        "mdash" => "—",
        "ndash" => "–",
        "laquo" => "«",
        "raquo" => "»",
        "auml" => "ä",
        "ouml" => "ö",
        "uuml" => "ü",
        "Auml" => "Ä",
        "Ouml" => "Ö",
        "Uuml" => "Ü",
        "szlig" => "ß",
        "euro" => "€",
        "deg" => "°",
        "copy" => "©",
        "reg" => "®",
        "shy" => "",
        _ => return None,
    };
    Some(value.to_string())
}
