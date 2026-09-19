//! Schlussanfrage der Websuche und das Sicherheitsnetz gegen Antworten, die
//! einen Tool-Aufruf nur als Text enthalten (Etappe 2c). Aus `chat.rs`
//! ausgelagert (Modulgroesse), Verhalten unveraendert.

use crate::ai::client as ai_client;
use crate::ai::client::ChatMessage;
use crate::ai::tool_stream;
use crate::summarize::SummaryTarget;

/// Hoechstens so viele Unicode-Skalare duerfen vor einem `|DSML|`-Block stehen.
const MAX_DSML_PREAMBLE_CHARS: usize = 200;

/// Hoechstens so viele Provider-Requests darf eine Frage insgesamt ausloesen
/// (5 Tool-Runden + Schlussanfrage + hoechstens ein Rueckfall/Wiederholung).
pub(crate) const MAX_PROVIDER_REQUESTS: usize = 7;

/// Meldung, wenn keine verwertbare Antwort mehr zustande kommt.
pub(crate) const EMPTY_FINAL_ANSWER_MESSAGE: &str =
    "Das Modell hat nach der Recherche keine Antwort geliefert – bitte erneut versuchen";

/// Zaehlt die Provider-Aufrufe einer Frage; ohne Restbudget gibt es keinen
/// weiteren Versuch mehr.
#[derive(Debug)]
pub(crate) struct RequestBudget {
    remaining: usize,
}

impl RequestBudget {
    pub(crate) fn new(max: usize) -> Self {
        Self { remaining: max }
    }

    pub(crate) fn take(&mut self) -> Result<(), String> {
        if self.remaining == 0 {
            return Err(EMPTY_FINAL_ANSWER_MESSAGE.to_string());
        }
        self.remaining -= 1;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn remaining(&self) -> usize {
        self.remaining
    }
}

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

/// Anzahl der Unicode-Skalare vor dem ersten `|DSML|` (Leerraum-tolerant).
/// Gezaehlt werden Skalare, nicht Bytes (Multibyte-Text vor dem Muster darf
/// weder panisch werden noch die Grenze verzerren).
fn first_dsml_position(text: &str) -> Option<usize> {
    let lowered = text.to_ascii_lowercase();
    let mut compact = String::with_capacity(lowered.len());
    for ch in lowered.chars() {
        if !ch.is_whitespace() {
            compact.push(ch);
        }
    }
    // `find` liefert einen Byte-Offset; die Grenze zaehlt Unicode-Skalare.
    let byte_position = compact.find("|dsml|")?;
    Some(compact[..byte_position].chars().count())
}

/// Entfernt Markdown-Codebloecke und Inline-Code (legitime Zitate). Ein
/// **ungeschlossener** Backtick oder Zaun ist kein Code: das Zeichen wird
/// uebersprungen, der Rest bleibt erhalten (sonst verschwaende Markup dahinter).
fn strip_code(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let Some(start) = rest.find("```") else {
            break;
        };
        match rest[start + 3..].find("```") {
            Some(end) => {
                out.push_str(&rest[..start]);
                rest = &rest[start + 3 + end + 3..];
            }
            None => {
                // Kein Abschluss: nur den Zaun ueberspringen, der Rest bleibt.
                out.push_str(&rest[..start]);
                rest = &rest[start + 3..];
                continue;
            }
        }
    }
    out.push_str(rest);

    // Inline-Code: `...` (jeweils paarweise); ungeschlossene Backticks bleiben
    // als Text erhalten.
    let mut result = String::with_capacity(out.len());
    let mut remainder = out.as_str();
    loop {
        let Some(start) = remainder.find('`') else {
            break;
        };
        match remainder[start + 1..].find('`') {
            Some(end) => {
                result.push_str(&remainder[..start]);
                remainder = &remainder[start + 1 + end + 1..];
            }
            None => {
                // Kein Abschluss: nur den Backtick ueberspringen.
                result.push_str(&remainder[..start]);
                remainder = &remainder[start + 1..];
                continue;
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
/// wird auf die Form ohne `tools` zurueckgefallen; der Rueckfall wird gemerkt,
/// damit spaetere Schlussanfragen dieser Frage keinen erneuten Fehlversuch
/// kosten. Jeder Provider-Aufruf verbraucht ein Budget aus
/// `MAX_PROVIDER_REQUESTS`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn final_round_request(
    http: &reqwest::Client,
    target: &SummaryTarget,
    messages: &[ChatMessage],
    tools: &[serde_json::Value],
    tool_choice_supported: &mut bool,
    budget: &mut RequestBudget,
    on_delta: &mut impl FnMut(&str),
    is_cancelled: &mut impl FnMut() -> bool,
) -> Result<tool_stream::ChatTurn, String> {
    if !*tool_choice_supported {
        return send_without_tools(http, target, messages, budget, on_delta, is_cancelled).await;
    }

    budget.take()?;
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
                *tool_choice_supported = false;
                send_without_tools(http, target, messages, budget, on_delta, is_cancelled).await
            } else {
                Err(error.to_string())
            }
        }
    }
}

/// Rueckfallform der Schlussanfrage: ohne `tools`, ohne `tool_choice`.
async fn send_without_tools(
    http: &reqwest::Client,
    target: &SummaryTarget,
    messages: &[ChatMessage],
    budget: &mut RequestBudget,
    on_delta: &mut impl FnMut(&str),
    is_cancelled: &mut impl FnMut() -> bool,
) -> Result<tool_stream::ChatTurn, String> {
    budget.take()?;
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
}
