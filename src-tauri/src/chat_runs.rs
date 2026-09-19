//! Laufregister des Video-Chats: hoechstens eine Anfrage pro Video und eine
//! Abbruchmoeglichkeit je `request_id`. Aus `chat.rs` ausgelagert (Modulgroesse),
//! Verhalten unveraendert.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::storage::AppResult;

/// Vorab abgebrochene Anfragen ohne Lauf verfallen nach dieser Zeit.
const PENDING_RUN_MAX_AGE: Duration = Duration::from_secs(60);

#[derive(Debug)]
struct RunEntry {
    video_id: Option<i64>,
    flag: Arc<AtomicBool>,
    created: Instant,
}

/// Laufregister des Video-Chats: hoechstens eine Anfrage pro Video, jede
/// Anfrage ueber ihre `request_id` abbrechbar.
#[derive(Debug, Clone, Default)]
pub struct ChatRuns {
    entries: Arc<Mutex<HashMap<String, RunEntry>>>,
}

impl ChatRuns {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, RunEntry>> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Erste Aktion von `chat_send`: prueft und reserviert den Lauf.
    pub fn begin(&self, request_id: &str, video_id: i64) -> AppResult<ChatRunGuard> {
        if request_id.trim().is_empty() {
            return Err("Ungültige Anfrage-ID".to_string());
        }
        let mut entries = self.lock();
        entries.retain(|_, entry| {
            entry.video_id.is_some() || entry.created.elapsed() < PENDING_RUN_MAX_AGE
        });
        if let Some(entry) = entries.get(request_id) {
            if entry.video_id.is_none() {
                entries.remove(request_id);
                return Err("KI-Antwort abgebrochen".to_string());
            }
            return Err("Anfrage-ID bereits in Verwendung".to_string());
        }
        if entries
            .values()
            .any(|entry| entry.video_id == Some(video_id))
        {
            return Err("Es läuft bereits eine Chat-Anfrage für dieses Video".to_string());
        }
        let flag = Arc::new(AtomicBool::new(false));
        entries.insert(
            request_id.to_string(),
            RunEntry {
                video_id: Some(video_id),
                flag: flag.clone(),
                created: Instant::now(),
            },
        );
        Ok(ChatRunGuard {
            runs: self.clone(),
            request_id: request_id.to_string(),
            flag,
        })
    }

    /// Bricht eine bekannte Anfrage ab. Unbekannte IDs werden als vorab
    /// abgebrochen vermerkt, damit ein spaeterer `chat_send` sofort stoppt.
    pub fn cancel(&self, request_id: &str) {
        let mut entries = self.lock();
        match entries.get_mut(request_id) {
            Some(entry) => entry.flag.store(true, Ordering::SeqCst),
            None => {
                entries.insert(
                    request_id.to_string(),
                    RunEntry {
                        video_id: None,
                        flag: Arc::new(AtomicBool::new(true)),
                        created: Instant::now(),
                    },
                );
            }
        }
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }
}

/// Entfernt beim Verlassen den eigenen Registereintrag, aber nur wenn dort
/// noch das eigene Flag steht (kein Entfernen fremder Laeufe).
#[derive(Debug)]
pub struct ChatRunGuard {
    runs: ChatRuns,
    request_id: String,
    flag: Arc<AtomicBool>,
}

impl ChatRunGuard {
    pub(crate) fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

impl Drop for ChatRunGuard {
    fn drop(&mut self) {
        let mut entries = self.runs.lock();
        let is_own = entries
            .get(&self.request_id)
            .is_some_and(|entry| Arc::ptr_eq(&entry.flag, &self.flag));
        if is_own {
            entries.remove(&self.request_id);
        }
    }
}
