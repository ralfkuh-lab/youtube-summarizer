//! Schlussanfrage der Websuche und das Sicherheitsnetz gegen Antworten, die
//! einen Tool-Aufruf nur als Text enthalten (Etappe 2c). Aus `chat.rs`
//! ausgelagert (Modulgroesse), Verhalten unveraendert.

use crate::ai::client as ai_client;
use crate::ai::client::ChatMessage;
use crate::ai::tool_stream;
use crate::summarize::SummaryTarget;

/// Hoechstens so viele Zeichen duerfen vor einem `|DSML|`-Block stehen.
const MAX_DSML_PREAMBLE_CHARS: usize = 200;

/// Sicherheitsnetz: erkennt Antworten, die einen Tool-Aufruf in modell-
/// interner Syntax als Text enthalten (statt echter `tool_calls`). Reine
/// Funktion, Gross-/Kleinschreibung egal, Leerraum wird entfernt.
/// Beginnt der Text (nach Trimmen) mit einem der Muster? Leerraum innerhalb
/// des Musters ist egal (der Praxistext schreibt ` < | DSML | calls>`).
fn starts_with_markup(text: &str) -> bool {
    let head: String = text
        .chars()
        .take(64)
        .filter(|ch| !ch.is_whitespace())
        .collect();
    let head = head.to_ascii_lowercase();
    [
        "<|dsml|",
        "<tool_call",
        "<|tool▁calls",
        "<|tool_calls",
        "<function_calls",
        "<invokename=",
    ]
    .iter()
    .any(|pattern| head.starts_with(pattern))
}

/// Position des ersten `|DSML|` (Leerraum-tolerant) im Text.
fn first_dsml_position(text: &str) -> Option<usize> {
    let lowered = text.to_ascii_lowercase();
    let mut compact = String::with_capacity(lowered.len());
    let mut positions = Vec::with_capacity(lowered.len());
    for (index, ch) in lowered.char_indices() {
        if !ch.is_whitespace() {
            compact.push(ch);
            positions.push(index);
        }
    }
    let position = compact.find("|dsml|")?;
    Some(positions[position])
}

/// Entfernt Markdown-Codebloecke und Inline-Code (legitime Zitate).
fn strip_code(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let Some(start) = rest.find("```") else {
            break;
        };
        out.push_str(&rest[..start]);
        match rest[start + 3..].find("```") {
            Some(end) => rest = &rest[start + 3 + end + 3..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);

    // Inline-Code: `...` (jeweils paarweise).
    let mut result = String::with_capacity(out.len());
    let mut remainder = out.as_str();
    while let Some(start) = remainder.find('`') {
        result.push_str(&remainder[..start]);
        match remainder[start + 1..].find('`') {
            Some(end) => remainder = &remainder[start + 1 + end + 1..],
            None => {
                remainder = "";
                break;
            }
        }
    }
    result.push_str(remainder);
    result
}

/// Sicherheitsnetz: erkennt Antworten, die einen Tool-Aufruf in modell-
/// interner Syntax als Text enthalten (statt echter `tool_calls`). Reine
/// Funktion, Gross-/Kleinschreibung egal.
///
/// Wichtig (Nachbesserung N1): Ein Treffer mitten in einem Erklaertext ist
/// **kein** Markup - die Videos handeln oft von Tool-Calling, korrekte Antworten
/// zitieren die Syntax. Erkannt wird daher nur:
/// 1. der Text **beginnt** mit einem Muster,
/// 2. der Text besteht vollstaendig aus einem JSON-Objekt mit `name` und
///    `arguments`, oder
/// 3. `|DSML|` kommt vor und davor stehen weniger als 200 Zeichen.
/// Markdown-Codebloecke und Inline-Code werden vorher entfernt.
pub fn looks_like_tool_markup(text: &str) -> bool {
    let stripped = strip_code(text);
    let trimmed = stripped.trim();
    if trimmed.is_empty() {
        return false;
    }
    if starts_with_markup(trimmed) {
        return true;
    }
    // Ausschliesslich ein JSON-Objekt mit `name` und `arguments`.
    if let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if map.contains_key("name") && map.contains_key("arguments") {
            return true;
        }
    }
    // Kurzer Vorspann vor einem DSML-Block ("Ich hole die Seite. <|DSML|calls>").
    first_dsml_position(trimmed).is_some_and(|position| position < MAX_DSML_PREAMBLE_CHARS)
}

/// Schlussanfrage der Websuche: `tools` + `tool_choice: "none"` und eine nicht
/// gespeicherte Abschluss-Nachricht. Antwortet der Provider mit HTTP 400/422,
/// wird einmalig auf die Form ohne `tools` zurueckgefallen.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn final_round_request(
    http: &reqwest::Client,
    target: &SummaryTarget,
    messages: &[ChatMessage],
    tools: &[serde_json::Value],
    on_delta: &mut impl FnMut(&str),
    is_cancelled: &mut impl FnMut() -> bool,
) -> Result<tool_stream::ChatTurn, String> {
    match tool_stream::chat_stream_with_tools_and_choice(
        http,
        &target.base_url,
        target.api_key.as_deref(),
        &target.model,
        messages,
        tools,
        Some("none"),
        &mut *on_delta,
        &mut *is_cancelled,
    )
    .await
    {
        Ok(turn) => Ok(turn),
        // Leerer Text ist in der Schlussanfrage kein harter Client-Fehler,
        // sondern loest das Sicherheitsnetz aus.
        Err(ai_client::ChatError::MissingChoice) => Ok(tool_stream::ChatTurn {
            content: String::new(),
            tool_calls: Vec::new(),
        }),
        Err(error) => {
            if matches!(
                error,
                ai_client::ChatError::Http { status, .. }
                    if status.as_u16() == 400 || status.as_u16() == 422
            ) {
                eprintln!("tool_choice wird nicht unterstuetzt, Rueckfall ohne tools: {error}");
                let text = ai_client::chat_stream_cancellable(
                    http,
                    &target.base_url,
                    target.api_key.as_deref(),
                    &target.model,
                    messages,
                    &mut *on_delta,
                    &mut *is_cancelled,
                )
                .await
                .map_err(|error| error.to_string())?;
                Ok(tool_stream::ChatTurn {
                    content: text,
                    tool_calls: Vec::new(),
                })
            } else {
                Err(error.to_string())
            }
        }
    }
}
