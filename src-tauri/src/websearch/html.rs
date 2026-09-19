//! Grobe HTML-zu-Text-Umwandlung ohne zusaetzliche Crate.

// -------------------------------------------------------------- HTML-Text --

/// Grobe HTML-zu-Text-Umwandlung ohne zusaetzliche Crate: `script`/`style`/
/// `noscript` samt Inhalt entfernen, Tags strippen, Entities dekodieren,
/// Whitespace normalisieren.
pub fn html_to_text(html: &str) -> String {
    let without_blocks = remove_elements(html, &["script", "style", "noscript"]);
    normalize_whitespace(&decode_entities(&strip_tags(&without_blocks)))
}

fn remove_elements(html: &str, names: &[&str]) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut index = 0usize;
    while index < html.len() {
        let rest = &html[index..];
        if let Some(ch) = rest.chars().next() {
            if ch == '<' {
                let name_end = lower[index + 1..]
                    .find(|c: char| !c.is_ascii_alphanumeric())
                    .map(|offset| index + 1 + offset)
                    .unwrap_or(html.len());
                let tag = &lower[index + 1..name_end];
                if names.contains(&tag) {
                    let closing = format!("</{tag}");
                    match lower[name_end..].find(&closing) {
                        Some(offset) => {
                            let after_close = name_end + offset + closing.len();
                            index = lower[after_close..]
                                .find('>')
                                .map(|position| after_close + position + 1)
                                .unwrap_or(html.len());
                        }
                        // Ohne Abschluss-Tag wird der Rest verworfen.
                        None => index = html.len(),
                    }
                    continue;
                }
            }
            out.push(ch);
            index += ch.len_utf8();
        }
    }
    out
}

fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
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

fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
