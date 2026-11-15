//! Playlists and albums kept on the device: which are on, since when, and the songs each held at
//! its last sync. [`crate::downloads::DownloadManager`] keeps them in step with the downloads.

use std::collections::{HashMap, HashSet};

use rusqlite::{OptionalExtension, params};

use super::Library;
use crate::error::Result;

/// What kind of collection is kept on the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CollectionKind {
    Playlist,
    Album,
}

impl CollectionKind {
    pub fn id(self) -> &'static str {
        match self {
            CollectionKind::Playlist => "playlist",
            CollectionKind::Album => "album",
        }
    }

    pub fn from_id(id: &str) -> Option<CollectionKind> {
        match id {
            "playlist" => Some(CollectionKind::Playlist),
            "album" => Some(CollectionKind::Album),
            _ => None,
        }
    }
}

/// A playlist or album whose Download switch is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncedCollection {
    pub kind: CollectionKind,
    pub id: String,
    /// When the switch was turned on, in Unix milliseconds.
    pub since_ms: i64,
    /// Set once every song was on the device, so later additions read as syncing.
    pub complete: bool,
}

/// The songs a collection has in the library right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CollectionSongs {
    /// The playlist or album isn't in the library any more.
    Gone,
    /// A playlist whose songs haven't been fetched yet; the list entry says how many it has.
    Unknown { expected: u32 },
    /// Song ids in the collection's order, each once.
    Known(Vec<String>),
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

fn unique(ids: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    ids.into_iter().filter(|id| seen.insert(id.clone())).collect()
}

impl Library {
    /// Collections with the Download switch on, the most recently turned on first.
    pub fn synced_collections(&self) -> Result<Vec<SyncedCollection>> {
        self.read(|c| {
            let mut st = c.prepare_cached("SELECT kind, id, since_ms, complete FROM synced_collection ORDER BY since_ms DESC, kind, id")?;
            let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, bool>(3)?)))?;
            let mut out = Vec::new();
            for row in rows {
                let (kind, id, since_ms, complete) = row?;
                if let Some(kind) = CollectionKind::from_id(&kind) {
                    out.push(SyncedCollection { kind, id, since_ms, complete });
                }
            }
            Ok(out)
        })
    }

    /// One collection's record, if its switch is on.
    pub fn synced_collection(&self, kind: CollectionKind, id: &str) -> Result<Option<SyncedCollection>> {
        self.read(|c| {
            Ok(c.query_row("SELECT since_ms, complete FROM synced_collection WHERE kind = ?1 AND id = ?2", params![kind.id(), id], |r| Ok(SyncedCollection { kind, id: id.to_string(), since_ms: r.get(0)?, complete: r.get(1)? })).optional()?)
        })
    }

    /// Turns a collection on. Returns false if it already was.
    pub(crate) fn add_synced_collection(&self, kind: CollectionKind, id: &str) -> Result<bool> {
        self.write_quiet(|tx| Ok(tx.execute("INSERT OR IGNORE INTO synced_collection(kind, id, since_ms, complete) VALUES (?1, ?2, ?3, 0)", params![kind.id(), id, now_ms()])? > 0))
    }

    /// Turns a collection off and returns the songs it held.
    pub(crate) fn remove_synced_collection(&self, kind: CollectionKind, id: &str) -> Result<Vec<String>> {
        let members = self.collection_members(kind, id)?;
        self.write_quiet(|tx| {
            tx.execute("DELETE FROM synced_collection WHERE kind = ?1 AND id = ?2", params![kind.id(), id])?;
            Ok(())
        })?;
        Ok(members)
    }

    /// Turns every collection off.
    pub(crate) fn clear_synced_collections(&self) -> Result<()> {
        self.write_quiet(|tx| {
            tx.execute("DELETE FROM synced_collection", [])?;
            Ok(())
        })
    }

    pub(crate) fn set_collection_complete(&self, kind: CollectionKind, id: &str, complete: bool) -> Result<()> {
        self.write_quiet(|tx| {
            tx.execute("UPDATE synced_collection SET complete = ?3 WHERE kind = ?1 AND id = ?2", params![kind.id(), id, complete])?;
            Ok(())
        })
    }

    /// The songs a collection held at its last sync.
    pub fn collection_members(&self, kind: CollectionKind, id: &str) -> Result<Vec<String>> {
        self.read(|c| {
            let mut st = c.prepare_cached("SELECT song_id FROM collection_song WHERE kind = ?1 AND id = ?2")?;
            let rows = st.query_map(params![kind.id(), id], |r| r.get::<_, String>(0))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    pub(crate) fn set_collection_members(&self, kind: CollectionKind, id: &str, songs: &[String]) -> Result<()> {
        self.write_quiet(|tx| {
            tx.execute("DELETE FROM collection_song WHERE kind = ?1 AND id = ?2", params![kind.id(), id])?;
            let mut st = tx.prepare_cached("INSERT OR IGNORE INTO collection_song(kind, id, song_id) VALUES (?1, ?2, ?3)")?;
            for song in songs {
                st.execute(params![kind.id(), id, song])?;
            }
            Ok(())
        })
    }

    /// The songs a playlist or album has in the library now.
    pub fn collection_songs(&self, kind: CollectionKind, id: &str) -> Result<CollectionSongs> {
        self.read(|c| match kind {
            CollectionKind::Album => {
                let exists: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM album WHERE id = ?1)", [id], |r| r.get(0))?;
                if !exists {
                    return Ok(CollectionSongs::Gone);
                }
                let mut st = c.prepare_cached("SELECT id FROM song WHERE album_id = ?1 ORDER BY disc, track, sort_name")?;
                let rows = st.query_map([id], |r| r.get::<_, String>(0))?;
                Ok(CollectionSongs::Known(unique(rows.collect::<std::result::Result<Vec<_>, _>>()?)))
            }
            CollectionKind::Playlist => {
                let count: Option<i64> = c.query_row("SELECT song_count FROM playlist WHERE id = ?1", [id], |r| r.get(0)).optional()?;
                let Some(count) = count else { return Ok(CollectionSongs::Gone) };
                let mut st = c.prepare_cached("SELECT song_id FROM playlist_song WHERE playlist_id = ?1 ORDER BY position")?;
                let rows = st.query_map([id], |r| r.get::<_, String>(0))?;
                let songs = unique(rows.collect::<std::result::Result<Vec<_>, _>>()?);
                // An empty list with a count means the playlist's songs haven't been fetched yet.
                if songs.is_empty() && count > 0 {
                    return Ok(CollectionSongs::Unknown { expected: count as u32 });
                }
                Ok(CollectionSongs::Known(songs))
            }
        })
    }

    /// Whether a playlist's songs still need fetching before it can be kept on the device.
    pub fn collection_needs_fetch(&self, kind: CollectionKind, id: &str) -> Result<bool> {
        Ok(matches!(self.collection_songs(kind, id)?, CollectionSongs::Unknown { .. }))
    }

    /// Songs some collection holds, other than the one named.
    pub(crate) fn songs_held_by_others(&self, kind: CollectionKind, id: &str) -> Result<HashSet<String>> {
        self.read(|c| {
            let mut st = c.prepare_cached("SELECT DISTINCT song_id FROM collection_song WHERE NOT (kind = ?1 AND id = ?2)")?;
            let rows = st.query_map(params![kind.id(), id], |r| r.get::<_, String>(0))?;
            Ok(rows.collect::<std::result::Result<HashSet<_>, _>>()?)
        })
    }

    /// Whether any collection holds this song, or it was downloaded on its own.
    pub fn is_song_held(&self, song_id: &str) -> Result<bool> {
        self.read(|c| {
            Ok(c.query_row(
                "SELECT EXISTS(SELECT 1 FROM collection_song WHERE song_id = ?1) OR EXISTS(SELECT 1 FROM download WHERE song_id = ?1 AND single = 1)",
                [song_id],
                |r| r.get(0),
            )?)
        })
    }

    /// The collections holding a song.
    pub fn collections_holding(&self, song_id: &str) -> Result<Vec<(CollectionKind, String)>> {
        self.read(|c| {
            let mut st = c.prepare_cached("SELECT kind, id FROM collection_song WHERE song_id = ?1")?;
            let rows = st.query_map([song_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            let mut out = Vec::new();
            for row in rows {
                let (kind, id) = row?;
                if let Some(kind) = CollectionKind::from_id(&kind) {
                    out.push((kind, id));
                }
            }
            Ok(out)
        })
    }

    /// Songs downloaded on their own, as songs.
    pub fn single_download_ids(&self) -> Result<Vec<String>> {
        self.read(|c| {
            let mut st = c.prepare_cached("SELECT song_id FROM download WHERE single = 1")?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Whether a song was downloaded on its own.
    pub fn is_single_download(&self, song_id: &str) -> Result<bool> {
        self.read(|c| Ok(c.query_row("SELECT single FROM download WHERE song_id = ?1", [song_id], |r| r.get::<_, bool>(0)).optional()?.unwrap_or(false)))
    }

    pub(crate) fn download_set_single(&self, song_ids: &[String], single: bool) -> Result<()> {
        self.write_quiet(|tx| {
            let mut st = tx.prepare_cached("UPDATE download SET single = ?2 WHERE song_id = ?1")?;
            for id in song_ids {
                st.execute(params![id, single])?;
            }
            Ok(())
        })
    }

    /// Which of these songs are downloaded, with their recorded sizes. One point lookup each.
    pub fn downloaded_among(&self, song_ids: &[String]) -> Result<HashMap<String, u64>> {
        self.read(|c| {
            let mut st = c.prepare_cached("SELECT size FROM download WHERE song_id = ?1")?;
            let mut out = HashMap::new();
            for id in song_ids {
                if let Some(size) = st.query_row([id], |r| r.get::<_, i64>(0)).optional()? {
                    out.insert(id.clone(), size as u64);
                }
            }
            Ok(out)
        })
    }

    /// Every song a kept collection holds or that was downloaded on its own.
    pub(crate) fn held_song_ids(&self) -> Result<HashSet<String>> {
        self.read(|c| {
            let mut st = c.prepare_cached("SELECT song_id FROM collection_song UNION SELECT song_id FROM download WHERE single = 1")?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?;
            Ok(rows.collect::<std::result::Result<HashSet<_>, _>>()?)
        })
    }
}
