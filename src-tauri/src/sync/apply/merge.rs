//! Apply von `videoGone`/`collectionGone merged` X → M (Spec-Tabelle, drei
//! `merged`-Zeilen).

use rusqlite::{params, OptionalExtension};
use sync_proto::{name_key, now_canonical};

use super::super::{db_error, privatize};
use super::Run;
use crate::storage::AppResult;

/// `(changed_at, seq)` eines ausstehenden Outbox-Eintrags; jünger = größer
/// (gleiche Ordnung wie beim Umschreiben).
type Pending = (String, i64);

/// Welche Seite beim Verschmelzen die LWW-Werte stellt: die mit ausstehendem
/// Eintrag, bei beiden der jüngere; ohne ausstehende Einträge M (das Echo
/// des Servers bringt dessen Stand).
fn x_wins(x: &Option<Pending>, m: &Option<Pending>) -> bool {
    match (x, m) {
        (Some(x), Some(m)) => x > m,
        (Some(_), None) => true,
        _ => false,
    }
}

/// Lokaler Videoinhalt in Spaltenreihenfolge von [`VIDEO_CONTENT`].
type Content = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<Vec<u8>>,
    Option<String>,
);

const VIDEO_CONTENT: &str = "title, url, thumbnail_url, published_at, transcript, chapters, \
     description, thumbnail_data, transcript_error";

fn empty_text(value: &Option<String>) -> bool {
    value.as_deref().is_none_or(str::is_empty)
}

/// Füllfeld wie auf dem Server: nie durch leer ersetzt, leeres aus der
/// anderen Seite gefüllt, zwei Werte → der Gewinner.
fn fill<T: Clone>(
    winner: &Option<T>,
    other: &Option<T>,
    empty: impl Fn(&Option<T>) -> bool,
) -> Option<T> {
    if empty(winner) && !empty(other) {
        other.clone()
    } else {
        winner.clone()
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Video,
    Collection,
}

impl Run<'_> {
    pub(super) fn video_merged(&mut self, x: &str, m: &str) -> AppResult<()> {
        let Some((x_id, _)) = self.video_row(x)? else {
            return Ok(());
        };
        match self.video_row(m)? {
            None => {
                // M hat einen ausstehenden Löschwunsch: der Wunsch bleibt, X
                // folgt ihm lokal.
                let wish: Option<Option<String>> = self
                    .conn
                    .query_row(
                        "SELECT tomb FROM sync_outbox WHERE entity = 'video' AND key = ?1",
                        params![m],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(db_error)?;
                match wish {
                    Some(Some(tomb)) if tomb == "withdrawn" => {
                        privatize::make_private(self.conn, x_id, false)?;
                    }
                    Some(_) => self.delete_video(x_id, x)?,
                    None => {
                        self.rewrite_outbox(Kind::Video, x, m)?;
                        self.execute("UPDATE videos SET uid = ?1 WHERE id = ?2", params![m, x_id])?;
                    }
                }
            }
            Some((m_id, _)) => {
                self.merge_video_content(x_id, m_id, x, m)?;
                self.execute(
                    "UPDATE summaries SET video_id = ?1 WHERE video_id = ?2",
                    params![m_id, x_id],
                )?;
                self.execute(
                    "UPDATE chats SET video_id = ?1 WHERE video_id = ?2",
                    params![m_id, x_id],
                )?;
                self.merge_pairs(Kind::Video, x_id, m_id, x, m)?;
                self.rewrite_outbox(Kind::Video, x, m)?;
                self.execute("DELETE FROM videos WHERE id = ?1", params![x_id])?;
                self.applied.video_ids.insert(m_id);
                self.applied.collections = true;
            }
        }
        self.applied.video_ids.insert(x_id);
        Ok(())
    }

    pub(super) fn collection_merged(&mut self, x: &str, m: &str) -> AppResult<()> {
        let Some(x_id) = self.collection(x)? else {
            return Ok(());
        };
        match self.collection(m)? {
            None if self.pending("collection", m)? => self.delete_collection(x_id, x)?,
            None => {
                self.rewrite_outbox(Kind::Collection, x, m)?;
                self.execute(
                    "UPDATE collections SET uid = ?1 WHERE id = ?2",
                    params![m, x_id],
                )?;
            }
            Some(m_id) => {
                let x_name: String = self
                    .conn
                    .query_row(
                        "SELECT name FROM collections WHERE id = ?1",
                        params![x_id],
                        |row| row.get(0),
                    )
                    .map_err(db_error)?;
                let rename = x_wins(
                    &self.pending_entry("collection", x)?,
                    &self.pending_entry("collection", m)?,
                );
                self.merge_pairs(Kind::Collection, x_id, m_id, x, m)?;
                self.rewrite_outbox(Kind::Collection, x, m)?;
                self.execute("DELETE FROM collections WHERE id = ?1", params![x_id])?;
                if rename {
                    self.take_collection_name(m_id, &x_name)?;
                }
            }
        }
        self.applied.collections = true;
        Ok(())
    }

    fn pending_entry(&self, entity: &str, key: &str) -> AppResult<Option<Pending>> {
        self.conn
            .query_row(
                "SELECT changed_at, seq FROM sync_outbox WHERE entity = ?1 AND key = ?2",
                params![entity, key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(db_error)
    }

    /// Fachinhalt von X und M zusammenführen, bevor X verschwindet: Der
    /// Snapshot baut Operationen aus der Zeile, eine ungesendete Änderung an X
    /// muss also in M stehen. Regeln wie auf dem Server (LWW-Felder vom
    /// Gewinner, Füllfelder nie geleert, `transcript_error` leer bei
    /// Transkript).
    fn merge_video_content(&self, x_id: i64, m_id: i64, x: &str, m: &str) -> AppResult<()> {
        let read = |id: i64| -> AppResult<Content> {
            self.conn
                .query_row(
                    &format!("SELECT {VIDEO_CONTENT} FROM videos WHERE id = ?1"),
                    params![id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                            row.get(6)?,
                            row.get(7)?,
                            row.get(8)?,
                        ))
                    },
                )
                .map_err(db_error)
        };
        let (from_x, from_m) = (read(x_id)?, read(m_id)?);
        let x_first = x_wins(
            &self.pending_entry("video", x)?,
            &self.pending_entry("video", m)?,
        );
        let (winner, other) = if x_first {
            (&from_x, &from_m)
        } else {
            (&from_m, &from_x)
        };
        let transcript = fill(&winner.4, &other.4, empty_text);
        let transcript_error = if empty_text(&transcript) {
            winner.8.clone()
        } else {
            None
        };
        self.execute(
            "UPDATE videos SET title = ?1, url = ?2, thumbnail_url = ?3, published_at = ?4, \
             transcript = ?5, chapters = ?6, description = ?7, thumbnail_data = ?8, \
             transcript_error = ?9 WHERE id = ?10",
            params![
                winner.0,
                winner.1,
                winner.2,
                winner.3,
                transcript,
                fill(&winner.5, &other.5, empty_text),
                fill(&winner.6, &other.6, empty_text),
                fill(&winner.7, &other.7, |value: &Option<Vec<u8>>| {
                    value.as_ref().is_none_or(Vec::is_empty)
                }),
                transcript_error,
                m_id
            ],
        )?;
        Ok(())
    }

    /// Übernimmt einen ausstehenden Namen von X für M, sofern er lokal keinen
    /// anderen Namen verletzt; sonst bleibt der Name von M.
    fn take_collection_name(&self, m_id: i64, name: &str) -> AppResult<()> {
        let taken: bool = {
            let mut stmt = self
                .conn
                .prepare("SELECT name FROM collections WHERE id != ?1")
                .map_err(db_error)?;
            let names = stmt
                .query_map(params![m_id], |row| row.get::<_, String>(0))
                .map_err(db_error)?;
            let mut taken = false;
            for other in names {
                taken |= name_key(&other.map_err(db_error)?) == name_key(name);
            }
            taken
        };
        if taken {
            eprintln!(
                "Sync: ausstehender Sammlungsname kollidiert lokal beim Verschmelzen, der Name der Wurzel bleibt"
            );
            return Ok(());
        }
        self.execute(
            "UPDATE collections SET name = ?1, updated_at = ?2 WHERE id = ?3",
            params![name, now_canonical(), m_id],
        )?;
        Ok(())
    }

    /// Hängt die Zuordnungen von X an M. Für jeden Partner (Sammlung bzw.
    /// Video) bestimmt ein ausstehender Eintrag die Anwesenheit des Zielpaars
    /// aus seinem eigenen lokalen Paar; stehen beide Seiten aus, entscheidet
    /// der jüngere Eintrag. Ohne ausstehende Einträge gilt die Vereinigung.
    /// So wird ein ungesendeter Entfernungswunsch nie zu `present = true`.
    fn merge_pairs(&self, kind: Kind, x_id: i64, m_id: i64, x: &str, m: &str) -> AppResult<()> {
        // Partner aus lokalen Paaren und aus ausstehenden Einträgen (auch
        // Entfernungswünsche ohne Paarzeile).
        let (rows_sql, entries_sql, partner_sql) = match kind {
            Kind::Video => (
                "SELECT c.uid FROM video_collections vc JOIN collections c ON c.id = vc.collection_id \
                 WHERE vc.video_id IN (?1, ?2)",
                "SELECT parent FROM sync_outbox WHERE entity = 'membership' AND owner IN (?3, ?4)",
                "SELECT id FROM collections WHERE uid = ?1",
            ),
            Kind::Collection => (
                "SELECT v.uid FROM video_collections vc JOIN videos v ON v.id = vc.video_id \
                 WHERE vc.collection_id IN (?1, ?2)",
                "SELECT owner FROM sync_outbox WHERE entity = 'membership' AND parent IN (?3, ?4)",
                "SELECT id FROM videos WHERE uid = ?1",
            ),
        };
        let partners: std::collections::BTreeSet<String> = {
            let mut stmt = self
                .conn
                .prepare(&format!("{rows_sql} UNION {entries_sql}"))
                .map_err(db_error)?;
            let rows = stmt
                .query_map(params![x_id, m_id, x, m], |row| {
                    row.get::<_, Option<String>>(0)
                })
                .map_err(db_error)?;
            let mut partners = std::collections::BTreeSet::new();
            for row in rows {
                partners.extend(row.map_err(db_error)?);
            }
            partners
        };
        for partner in partners {
            let partner_id: Option<i64> = self
                .conn
                .query_row(partner_sql, params![partner], |row| row.get(0))
                .optional()
                .map_err(db_error)?;
            let Some(partner_id) = partner_id else {
                continue;
            };
            // (Video-id, Sammlungs-id) und Zuordnungsschlüssel je Seite.
            let pair = |side_id: i64, side_uid: &str| match kind {
                Kind::Video => (side_id, partner_id, format!("{side_uid}/{partner}")),
                Kind::Collection => (partner_id, side_id, format!("{partner}/{side_uid}")),
            };
            let (x_video, x_collection, x_key) = pair(x_id, x);
            let (m_video, m_collection, m_key) = pair(m_id, m);
            let has = |video: i64, collection: i64| -> AppResult<bool> {
                self.conn
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM video_collections \
                         WHERE video_id = ?1 AND collection_id = ?2)",
                        params![video, collection],
                        |row| row.get(0),
                    )
                    .map_err(db_error)
            };
            let (x_has, m_has) = (has(x_video, x_collection)?, has(m_video, m_collection)?);
            let x_entry = self.pending_entry("membership", &x_key)?;
            let m_entry = self.pending_entry("membership", &m_key)?;
            let present = match (&x_entry, &m_entry) {
                (None, None) => x_has || m_has,
                _ if x_wins(&x_entry, &m_entry) => x_has,
                _ => m_has,
            };
            self.execute(
                "DELETE FROM video_collections WHERE video_id = ?1 AND collection_id = ?2",
                params![x_video, x_collection],
            )?;
            if present {
                self.execute(
                    "INSERT OR IGNORE INTO video_collections (video_id, collection_id, created_at) \
                     VALUES (?1, ?2, ?3)",
                    params![m_video, m_collection, now_canonical()],
                )?;
            } else {
                self.execute(
                    "DELETE FROM video_collections WHERE video_id = ?1 AND collection_id = ?2",
                    params![m_video, m_collection],
                )?;
            }
        }
        Ok(())
    }

    /// Schreibt Outbox-Einträge mit Schlüssel, Owner oder Parent `from` auf
    /// `to` um (Zuordnungsschlüssel eingeschlossen), `changed_at` und `seq`
    /// bleiben. Kollidiert der neue Schlüssel mit einem bestehenden Eintrag,
    /// bleibt der jüngere (`changed_at`, bei Gleichstand höhere `seq`).
    fn rewrite_outbox(&self, kind: Kind, from: &str, to: &str) -> AppResult<()> {
        let sql = match kind {
            Kind::Video => {
                "SELECT seq, entity, key, owner, parent, changed_at FROM sync_outbox \
                 WHERE (entity = 'video' AND key = ?1) OR owner = ?1"
            }
            Kind::Collection => {
                "SELECT seq, entity, key, owner, parent, changed_at FROM sync_outbox \
                 WHERE (entity = 'collection' AND key = ?1) \
                    OR (entity = 'membership' AND parent = ?1)"
            }
        };
        let entries: Vec<(i64, String, String, Option<String>, Option<String>, String)> = {
            let mut stmt = self.conn.prepare(sql).map_err(db_error)?;
            let rows = stmt
                .query_map(params![from], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                })
                .map_err(db_error)?;
            rows.collect::<Result<_, _>>().map_err(db_error)?
        };
        let swap = |value: &str| {
            if value == from {
                to.to_string()
            } else {
                value.to_string()
            }
        };
        for (seq, entity, key, owner, parent, changed_at) in entries {
            let new_key = match key.split_once('/') {
                Some((video, collection)) if entity == "membership" => {
                    format!("{}/{}", swap(video), swap(collection))
                }
                _ => swap(&key),
            };
            let other: Option<(i64, String)> = self
                .conn
                .query_row(
                    "SELECT seq, changed_at FROM sync_outbox \
                     WHERE entity = ?1 AND key = ?2 AND seq != ?3",
                    params![entity, new_key, seq],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(db_error)?;
            if let Some((other_seq, other_changed_at)) = other {
                if (other_changed_at.as_str(), other_seq) > (changed_at.as_str(), seq) {
                    self.execute("DELETE FROM sync_outbox WHERE seq = ?1", params![seq])?;
                    continue;
                }
                self.execute("DELETE FROM sync_outbox WHERE seq = ?1", params![other_seq])?;
            }
            self.execute(
                "UPDATE sync_outbox SET key = ?1, owner = ?2, parent = ?3 WHERE seq = ?4",
                params![
                    new_key,
                    owner.as_deref().map(swap),
                    parent.as_deref().map(swap),
                    seq
                ],
            )?;
        }
        Ok(())
    }
}
