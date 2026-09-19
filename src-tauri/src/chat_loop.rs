//! Bausteine der Chat-Schleife (Etappe 2/4): Live-Ereignisse der laufenden
//! Antwort und die Ausfuehrung eines Werkzeug-Aufrufs. Aus `chat.rs`
//! ausgelagert (Modulgroesse), Verhalten unveraendert.

use crate::ai::client as ai_client;
use crate::storage::AppResult;
use crate::websearch;

/// Meldung, wenn der Benutzer die Anfrage abbricht.
pub(crate) const CANCELLED_MESSAGE: &str = "KI-Antwort abgebrochen";

/// Hoechstens so viele Aufrufe werden pro Runde ausgefuehrt.
pub(crate) const MAX_TOOL_CALLS_PER_ROUND: usize = 4;
/// Laenge des an das Modell gemeldeten Fehlertexts in Unicode-Skalarwerten.
const MAX_TOOL_ERROR_CHARS: usize = 200;

/// Ein Live-Text-Ereignis der laufenden Antwort (Event `ai:chat_stream`): die
/// Provider-Runde, ihr bisheriger oder vollstaendiger Text und ob die Runde
/// damit abgeschlossen ist. `discarded` verwirft eine Runde wieder (verworfene
/// Markup-Antwort vor der Wiederholung).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatDelta<'a> {
    pub round: usize,
    pub text: &'a str,
    pub final_text: bool,
    pub discarded: bool,
}

/// Ein Live-Werkzeug-Ereignis (Event `ai:chat_tool`): die Runde, deren
/// Assistant-Turn den Aufruf ausgeloest hat, plus das Ereignis selbst.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatToolEvent {
    pub round: usize,
    pub event: websearch::ToolEvent,
}

/// Baut ein Werkzeug-Ereignis mit seiner Live-Runde.
pub(crate) fn tool_event(
    round: usize,
    kind: &'static str,
    label: String,
    status: &'static str,
) -> ChatToolEvent {
    ChatToolEvent {
        round,
        event: websearch::ToolEvent {
            kind,
            label,
            status,
        },
    }
}

/// Meldet das Ende einer Provider-Runde mit ihrem vollstaendigen Text; auch ein
/// leerer Text wird gemeldet, damit die Live-Anzeige die Runde abschliesst.
pub(crate) fn finish_round(on_delta: &mut impl FnMut(ChatDelta<'_>), round: usize, text: &str) {
    on_delta(ChatDelta {
        round,
        text,
        final_text: true,
        discarded: false,
    });
}

/// Meldet eine verworfene Runde: ihr Text verschwindet aus der Live-Anzeige.
pub(crate) fn discard_round(on_delta: &mut impl FnMut(ChatDelta<'_>), round: usize) {
    on_delta(ChatDelta {
        round,
        text: "",
        final_text: true,
        discarded: true,
    });
}

/// Prueft Tool-Name, Argumente und Rundenlimit, bevor der Executor laeuft.
/// Liefert bei Nichtausfuehrung den Modelltext und das feste Event-Label.
pub(crate) fn precheck_tool_call(
    index: usize,
    name: &str,
    arguments: &str,
) -> Result<(), (&'static str, &'static str)> {
    if index >= MAX_TOOL_CALLS_PER_ROUND {
        return Err((websearch::TOOL_LIMIT_MESSAGE, "Tool-Limit erreicht"));
    }
    let arguments_ok = match name {
        websearch::WEB_SEARCH_TOOL => websearch::string_argument(arguments, "query").is_ok(),
        websearch::FETCH_PAGE_TOOL => websearch::string_argument(arguments, "url").is_ok(),
        _ => return Err((websearch::UNKNOWN_TOOL_MESSAGE, "unbekanntes Tool")),
    };
    if !arguments_ok {
        return Err((websearch::INVALID_ARGUMENTS_MESSAGE, "ungültige Argumente"));
    }
    Ok(())
}

/// Laesst den Tool-Aufruf laufen und bricht ihn ab, wenn das Abbruch-Flag
/// gesetzt wird (Abfrage alle 250 ms).
pub(crate) async fn execute_with_cancel(
    runtime: &websearch::ToolRuntime,
    name: &str,
    arguments: &str,
    is_cancelled: &mut impl FnMut() -> bool,
) -> AppResult<Result<String, String>> {
    let running = (runtime.execute)(name.to_string(), arguments.to_string());
    tokio::pin!(running);
    loop {
        tokio::select! {
            result = &mut running => return Ok(result),
            _ = tokio::time::sleep(ai_client::CANCEL_POLL_INTERVAL) => {
                if is_cancelled() {
                    return Err(CANCELLED_MESSAGE.to_string());
                }
            }
        }
    }
}

/// Modelltext eines Fehlers: hoechstens 200 Skalarwerte, ohne Laeufe von drei
/// oder mehr `=` (damit sich keine Delimiter einschleusen lassen).
pub(crate) fn tool_error_text(reason: &str) -> String {
    let shortened: String = reason.trim().chars().take(MAX_TOOL_ERROR_CHARS).collect();
    let neutralized = neutralize_equals(&shortened);
    if neutralized.starts_with("Fehler:") {
        neutralized
    } else {
        format!("Fehler: {neutralized}")
    }
}

fn neutralize_equals(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut run = 0usize;
    for ch in text.chars() {
        if ch == '=' {
            run += 1;
            continue;
        }
        if run > 0 {
            out.extend(std::iter::repeat_n('=', run.min(2)));
            run = 0;
        }
        out.push(ch);
    }
    if run > 0 {
        out.extend(std::iter::repeat_n('=', run.min(2)));
    }
    out
}
