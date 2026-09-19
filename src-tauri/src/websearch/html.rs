//! Grobe HTML-zu-Text-Umwandlung ohne zusaetzliche Crate. Ein einziger
//! Vorwaertsdurchlauf (linear): es wird nie mehrfach erfolglos ueber den Rest
//! gesucht, sondern einmalig vorab ermittelt, ob es ueberhaupt noch ein `>`,
//! ein Kommentarende oder ein Abschluss-Tag gibt.

/// Blockelemente: ihr Anfangs-/End-Tag erzeugt einen Zeilenumbruch.
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

/// Elemente mit Rohtext-Inhalt (JavaScript/CSS): ohne Abschluss-Tag wird ihr
/// Rest bis zum naechsten `<` verworfen.
const RAW_TEXT_ELEMENTS: &[&str] = &["script", "style", "noscript"];

/// Seitengeruest: Inhalt wird verworfen, wenn ein Abschluss-Tag folgt; ohne
/// Abschluss-Tag bleibt der Inhalt erhalten.
const FURNITURE_ELEMENTS: &[&str] = &[
    "nav", "footer", "aside", "svg", "form", "button", "select", "template", "iframe",
];

/// Elemente, deren Inhalt nicht in den Modellkontext gehoert. In HTML ist keines
/// davon selbstschliessend.
const SKIPPED_ELEMENTS: &[&str] = &[
    "script", "style", "noscript", "nav", "footer", "aside", "svg", "form", "button", "select",
    "template", "iframe",
];

/// Reine Inline-Formatierungen trennen im Browser nicht: ihr Tag erzeugt kein
/// Leerzeichen (sonst wuerden Woerter auseinandergerissen).
const INLINE_ELEMENTS: &[&str] = &[
    "b", "i", "em", "strong", "u", "s", "sub", "sup", "span", "code", "mark", "small", "font",
    "abbr", "cite", "q", "time", "var", "kbd", "samp", "wbr",
];

/// `script`/`style`/`noscript` samt Inhalt, Kommentare und Tags entfernen,
/// Entities dekodieren, Blockumbrueche erhalten, Whitespace normalisieren.
pub fn html_to_text(html: &str) -> String {
    let lowered = html.to_ascii_lowercase();
    let bytes = html.as_bytes();
    // Vorabinformationen, damit "es gibt keinen Treffer mehr" in O(1) bekannt
    // ist (sonst waere der Durchlauf quadratisch).
    let last_gt = lowered.rfind('>');
    let last_comment_end = lowered.rfind("-->");
    let last_closing: Vec<Option<usize>> = SKIPPED_ELEMENTS
        .iter()
        .map(|name| lowered.rfind(&format!("</{name}")))
        .collect();

    let mut out = String::with_capacity(html.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'<' {
            match html[index..].find('<') {
                Some(offset) => {
                    out.push_str(&html[index..index + offset]);
                    index += offset;
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
            let after_open = &lowered[index + 4..];
            // `<!-->` und `<!--->` sind vollstaendige leere Kommentare.
            if after_open.starts_with('>') {
                index += 5;
                continue;
            }
            if after_open.starts_with("->") {
                index += 6;
                continue;
            }
            match last_comment_end.filter(|end| *end >= index) {
                Some(_) => {
                    let offset = lowered[index + 4..]
                        .find("-->")
                        .map(|offset| index + 4 + offset + 3)
                        .unwrap_or(bytes.len());
                    index = offset;
                }
                // Kein Kommentarende mehr: der Rest ist Kommentar.
                None => break,
            }
            continue;
        }

        // Tag-Ende anfuehrungsbewusst bestimmen. Gibt es gar kein '>' mehr, ist
        // der Rest Text - ohne erneutes Suchen.
        let tag_end = last_gt.filter(|position| *position >= index).and_then(|_| {
            // Bei unbalancierten Anfuehrungszeichen gibt es kein Ende
            // ausserhalb von Quotes: dann das naechste '>' nehmen, statt den
            // ganzen Rest als Text zu behandeln.
            find_tag_end(html, index)
                .or_else(|| html[index..].find('>').map(|offset| index + offset))
        });
        let Some(tag_end) = tag_end else {
            out.push_str(&html[index..]);
            break;
        };

        let tag = &lowered[index..=tag_end];
        let name = tag_name(tag);
        // Schliessende Tags sind keine Oeffner: `</nav>` darf keinen Skip
        // ausloesen.
        let is_closing = tag.starts_with("</");
        if BLOCK_ELEMENTS.contains(&name) {
            out.push('\n');
        } else if !INLINE_ELEMENTS.contains(&name) {
            // Entfernte Tags duerfen Woerter nicht verkleben ("Jahr2026").
            out.push(' ');
        }

        let skipped = if is_closing {
            None
        } else {
            SKIPPED_ELEMENTS.iter().position(|element| *element == name)
        };
        if let Some(position) = skipped {
            // In HTML ist keines dieser Elemente selbstschliessend: `<script/>`
            // oeffnet einen Rohtext-Block.
            let closing = format!("</{name}");
            // Nur suchen, wenn laut Vorabinfo ueberhaupt ein Abschluss-Tag folgt.
            let closing_start = last_closing[position]
                .filter(|start| *start > tag_end)
                .and_then(|_| {
                    lowered[tag_end + 1..]
                        .find(&closing)
                        .map(|offset| tag_end + 1 + offset)
                });
            match closing_start {
                Some(start) => {
                    let after_close = start + closing.len();
                    index = lowered[after_close..]
                        .find('>')
                        .map(|offset| after_close + offset + 1)
                        .unwrap_or(bytes.len());
                }
                // Unabgeschlossen: bei Rohtext (JavaScript/CSS) wird der Rest
                // bis zum naechsten '<' verworfen, bei Seitengeruest bleibt der
                // Inhalt erhalten (nur das oeffnende Tag entfaellt).
                None => {
                    if RAW_TEXT_ELEMENTS.contains(&name) {
                        index = html[tag_end + 1..]
                            .find('<')
                            .map(|offset| tag_end + 1 + offset)
                            .unwrap_or(bytes.len());
                    } else {
                        index = tag_end + 1;
                    }
                }
            }
            continue;
        }

        index = tag_end + 1;
    }

    normalize_text(&decode_entities(&out))
}

/// Position des Tag-Endes ab `start` ('<'): '>' innerhalb von "..." oder '...'
/// beendet das Tag nicht. `None`, wenn kein '>' ausserhalb von
/// Anfuehrungszeichen folgt.
fn find_tag_end(html: &str, start: usize) -> Option<usize> {
    let bytes = html.as_bytes();
    let mut index = start + 1;
    let mut quote: Option<u8> = None;
    while index < bytes.len() {
        let byte = bytes[index];
        match quote {
            Some(active) => {
                if byte == active {
                    quote = None;
                }
            }
            None => match byte {
                b'"' | b'\'' => quote = Some(byte),
                b'>' => return Some(index),
                _ => {}
            },
        }
        index += 1;
    }
    None
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
