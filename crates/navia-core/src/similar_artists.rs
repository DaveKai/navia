//! The artist page's Similar Artists, merging the server's similar artists with Deezer's related artists.
//! Only artists in the library are shown. Artists both sources name come first.
//! [`artist_page_info`] keeps the page's one `getArtistInfo2` a day per artist, which also carries the biography.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use tokio::time::Instant;

use crate::client::{Artist, ArtistInfo, Client};
use crate::deezer::Deezer;
use crate::engine::deezer_src;
use crate::library::Library;
use crate::library::taste::{local_day, now_unix};

/// Artists the section shows at most.
pub const SHOWN: usize = 20;
/// Similar artists asked of the server.
pub const SERVER_COUNT: u32 = 30;
/// Time for Deezer's lookups, past which the server's answer stands alone.
const DEEZER_TIME: Duration = Duration::from_secs(5);
/// Answers kept for the session before the oldest are dropped.
const KEPT: usize = 256;

type Kept = HashMap<String, (i64, ArtistInfo)>;
static ANSWERS: LazyLock<Mutex<Kept>> = LazyLock::new(Mutex::default);
static ASKING: LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = LazyLock::new(Mutex::default);

fn kept(artist_id: &str, day: i64) -> Option<ArtistInfo> {
    ANSWERS.lock().unwrap().get(artist_id).filter(|(d, _)| *d == day).map(|(_, info)| info.clone())
}

/// The artist's `getArtistInfo2` for today, asked once per artist and local day.
/// `None` when the ask fails. Failures aren't kept, so the next visit asks again.
pub async fn artist_page_info(client: &Client, db: &Library, artist_id: &str) -> Option<ArtistInfo> {
    let day = local_day(now_unix());
    if let Some(found) = kept(artist_id, day) {
        return Some(found);
    }
    let gate = ASKING.lock().unwrap().entry(artist_id.to_string()).or_default().clone();
    let _turn = gate.lock().await;
    if let Some(found) = kept(artist_id, day) {
        return Some(found);
    }
    let info = match client.artist_info2(artist_id, Some(SERVER_COUNT)).await {
        Ok(info) => info,
        Err(e) => {
            tracing::warn!("artist info for {artist_id}: {e}");
            ASKING.lock().unwrap().remove(artist_id);
            return None;
        }
    };
    let ids: Vec<String> = info.similar_artist.iter().map(|a| a.id.clone()).filter(|id| !id.is_empty()).collect();
    if let Err(e) = db.store_similar(artist_id, day, &ids) {
        tracing::warn!("couldn't keep similar artists for {artist_id}: {e}");
    }
    let mut answers = ANSWERS.lock().unwrap();
    if answers.len() >= KEPT {
        answers.clear();
    }
    answers.insert(artist_id.to_string(), (day, info.clone()));
    drop(answers);
    ASKING.lock().unwrap().remove(artist_id);
    Some(info)
}

/// Library artist ids Deezer relates to this artist, from the cache or a call. Empty with the gate closed.
pub async fn deezer_similar(db: &Library, deezer: &Deezer, artist_name: &str) -> Vec<String> {
    if !crate::deezer::gate(db).is_open() || artist_name.trim().is_empty() {
        return Vec::new();
    }
    let deadline = Instant::now() + DEEZER_TIME;
    let Some(id) = deezer_src::artist(db, deezer, artist_name, deadline).await else { return Vec::new() };
    deezer_src::related(db, deezer, id, deadline).await.iter().filter_map(|name| db.artist_by_name(name).ok().flatten()).map(|a| a.id).collect()
}

/// An artist's similar library artists for `day` from what's stored, without asking anyone.
/// `None` when the server hasn't been asked that day, so callers know to ask.
pub fn stored_for_day(db: &Library, artist_id: &str, day: i64) -> Option<Vec<String>> {
    let server = db.similar_for_day(artist_id, day).ok().flatten()?;
    if !crate::deezer::gate(db).is_open() {
        return Some(server);
    }
    let name = db.artist(artist_id).ok().flatten().map(|a| a.name).unwrap_or_default();
    let deezer: Vec<String> = deezer_src::kept_related(db, &name).iter().filter_map(|n| db.artist_by_name(n).ok().flatten()).map(|a| a.id).collect();
    Some(merge(&server, &deezer, artist_id, SERVER_COUNT as usize))
}

/// Merges both answers, artists both name first, then by best place with the server's first on a tie.
/// Leaves out the artist itself and repeats, and returns at most `limit`.
pub fn merge(server: &[String], deezer: &[String], own: &str, limit: usize) -> Vec<String> {
    // id -> (sources naming it, best place, server's place or after every server answer)
    let mut seen: HashMap<&str, (u8, usize, usize)> = HashMap::new();
    let mut order: Vec<&str> = Vec::new();
    for (list, from_server) in [(server, true), (deezer, false)] {
        let mut counted = std::collections::HashSet::new();
        for (place, id) in list.iter().enumerate() {
            if id.is_empty() || id == own || !counted.insert(id.as_str()) {
                continue;
            }
            let entry = seen.entry(id.as_str()).or_insert_with(|| {
                order.push(id.as_str());
                (0, usize::MAX, usize::MAX)
            });
            entry.0 += 1;
            entry.1 = entry.1.min(place);
            if from_server {
                entry.2 = place;
            }
        }
    }
    order.sort_by_key(|id| {
        let (sources, best, server_place) = seen[id];
        (std::cmp::Reverse(sources), best, server_place)
    });
    order.into_iter().take(limit).map(str::to_string).collect()
}

/// The section's artists as library rows in order. With `ask` false it never reaches the server or
/// Deezer and uses whatever was stored.
pub async fn similar_artists(client: &Client, db: &Library, deezer: &Deezer, artist_id: &str, ask: bool) -> Vec<(u32, Artist)> {
    let day = local_day(now_unix());
    let server: Vec<String> = match db.similar_for_day(artist_id, day).ok().flatten() {
        Some(ids) => ids,
        None if ask => match artist_page_info(client, db, artist_id).await {
            Some(_) => db.similar_for_day(artist_id, day).ok().flatten().unwrap_or_default(),
            // Unreachable, so the older answer stands.
            None => db.similar_stored(&[artist_id.to_string()]).ok().and_then(|mut m| m.remove(artist_id)).unwrap_or_default(),
        },
        None => db.similar_stored(&[artist_id.to_string()]).ok().and_then(|mut m| m.remove(artist_id)).unwrap_or_default(),
    };
    let deezer_ids = if ask {
        let name = db.artist(artist_id).ok().flatten().map(|a| a.name).unwrap_or_default();
        deezer_similar(db, deezer, &name).await
    } else {
        Vec::new()
    };
    let ids = merge(&server, &deezer_ids, artist_id, SHOWN);
    library_rows(db, &ids)
}

/// Library rows for artist ids, in the given order, leaving out ids the library doesn't have.
fn library_rows(db: &Library, ids: &[String]) -> Vec<(u32, Artist)> {
    let rowids = db.artist_rowids(ids).unwrap_or_default();
    let mut by_row: HashMap<u32, Artist> = db.artists_by_rowid(&rowids).unwrap_or_default().into_iter().collect();
    rowids.into_iter().filter_map(|r| by_row.remove(&r).map(|a| (r, a))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn agreement_comes_first_then_place_with_the_server_first_on_a_tie() {
        let server = ids(&["a", "b", "c", "d"]);
        let deezer = ids(&["x", "c", "y", "a"]);
        assert_eq!(merge(&server, &deezer, "me", 20), ids(&["a", "c", "x", "b", "y", "d"]));
    }

    #[test]
    fn leaves_out_the_artist_itself_repeats_and_blanks_and_keeps_the_limit() {
        let server = ids(&["me", "a", "a", "", "b"]);
        let deezer = ids(&["b", "c", "d"]);
        assert_eq!(merge(&server, &deezer, "me", 3), ids(&["b", "a", "c"]));
    }

    #[test]
    fn stored_merge_adds_deezers_kept_related_artists_only_with_the_switch_on() {
        use crate::client::MusicFolder;
        use crate::library::LibraryConfig;
        let dir = tempfile::tempdir().unwrap();
        let db = Library::open(&LibraryConfig { path: dir.path().join("lib.sqlite") }).unwrap();
        db.replace_libraries(&[MusicFolder { id: "1".into(), name: "L".into() }]).unwrap();
        let cur = db.begin_sync("1", None, false).unwrap();
        let artists: Vec<Artist> = ["me", "a", "b", "z"].iter().map(|id| Artist { id: (*id).into(), name: format!("Name {id}"), ..Default::default() }).collect();
        db.apply_artists("1", &cur, artists.len() as u32, &artists).unwrap();
        assert_eq!(stored_for_day(&db, "me", 5), None, "not asked today");
        db.store_similar("me", 5, &ids(&["a", "b"])).unwrap();
        crate::deezer::store(&db, &format!("artist:{}", crate::lanyard::matching::key("Name me")), "42", 1);
        crate::deezer::store(&db, "related:42", &serde_json::to_string(&["Name z", "Name b", "Not here"]).unwrap(), 1);
        assert_eq!(stored_for_day(&db, "me", 5), Some(ids(&["a", "b"])), "switch unset: the server's alone");
        crate::deezer::set_setting(&db, Some(true)).unwrap();
        assert_eq!(stored_for_day(&db, "me", 5), Some(ids(&["b", "a", "z"])), "named by both first, then by place");
        assert_eq!(stored_for_day(&db, "me", 6), None, "another day");
    }

    #[test]
    fn one_source_alone_keeps_its_order() {
        assert_eq!(merge(&ids(&["c", "a", "b"]), &[], "me", 20), ids(&["c", "a", "b"]));
        assert_eq!(merge(&[], &ids(&["z", "y"]), "me", 20), ids(&["z", "y"]));
        assert!(merge(&[], &[], "me", 20).is_empty());
    }
}
