//! Get Info for songs, albums, playlists and artists, from the JSON the library keeps per row.
//! Every field is optional, so a plain Subsonic server shows fewer rows. Missing totals are
//! derived from the library's songs.

use rusqlite::{OptionalExtension, params};
use serde_json::Value;

use super::Library;
use crate::client::{Album, Artist, ItemDate, Playlist, Song};
use crate::error::{Error, Result};

/// Credit roles in the order credits usually read. Roles not listed follow in the server's order.
pub const ROLE_ORDER: &[&str] = &["artist", "albumartist", "composer", "lyricist", "producer", "performer", "conductor", "arranger", "director", "engineer", "mixer", "remixer", "djmixer"];

/// Roles Get Info leaves out of the credits, because the title area already names them.
const ROLES_SHOWN_ELSEWHERE: &[&str] = &["artist", "albumartist"];

/// One name in a credit, with the library's artist id when the server gave one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CreditName {
    pub name: String,
    pub artist_id: Option<String>,
    /// OpenSubsonic `subRole`, for example the instrument of a performer.
    pub sub_role: Option<String>,
}

/// Everyone credited in one role, as OpenSubsonic `contributors` names it ("composer", ...).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Credit {
    pub role: String,
    pub names: Vec<CreditName>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReplayGainInfo {
    pub track_gain: Option<f64>,
    pub album_gain: Option<f64>,
    pub track_peak: Option<f64>,
    pub album_peak: Option<f64>,
}

/// A song's Get Info.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SongInfo {
    pub id: String,
    pub title: String,
    pub album: Option<String>,
    pub album_id: Option<String>,
    /// The display artist (or the plain artist field).
    pub artist: Option<String>,
    pub artist_id: Option<String>,
    /// OpenSubsonic `artists`, when the song credits more than one.
    pub artists: Vec<CreditName>,
    pub track: Option<u32>,
    pub disc: Option<u32>,
    pub year: Option<i32>,
    pub genres: Vec<String>,
    pub duration_s: Option<u32>,
    pub size: Option<u64>,
    pub suffix: Option<String>,
    pub content_type: Option<String>,
    pub bit_rate_kbps: Option<u32>,
    pub bit_depth: Option<u32>,
    pub sampling_rate: Option<u32>,
    pub channel_count: Option<u32>,
    /// As the server reports it: library-relative, or a made-up one when Navidrome hides paths.
    pub path: Option<String>,
    pub bpm: Option<u32>,
    pub replay_gain: Option<ReplayGainInfo>,
    pub isrc: Vec<String>,
    /// The recording's MusicBrainz id.
    pub music_brainz_id: Option<String>,
    pub comment: Option<String>,
    /// Credits other than artist and album artist, in [`ROLE_ORDER`].
    pub credits: Vec<Credit>,
    /// "explicit" or "clean"; `None` when unknown or not sent.
    pub explicit_status: Option<String>,
    /// When the server added it (ISO 8601).
    pub created: Option<String>,
    /// The signed-in user's own plays.
    pub play_count: Option<u64>,
    pub played: Option<String>,
    pub starred: Option<String>,
}

/// An album's Get Info.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AlbumInfo {
    pub id: String,
    pub name: String,
    pub artist: Option<String>,
    pub artist_id: Option<String>,
    pub artists: Vec<CreditName>,
    pub version: Option<String>,
    pub record_labels: Vec<String>,
    pub release_types: Vec<String>,
    pub release_date: Option<ItemDate>,
    pub original_release_date: Option<ItemDate>,
    pub year: Option<i32>,
    pub is_compilation: Option<bool>,
    pub music_brainz_id: Option<String>,
    pub genres: Vec<String>,
    pub explicit_status: Option<String>,
    pub song_count: Option<u32>,
    pub duration_s: Option<u64>,
    /// Discs, when there's more than one.
    pub disc_count: Option<u32>,
    /// Sum of its songs' file sizes in the library; `None` when none of them has a size.
    pub size: Option<u64>,
    pub created: Option<String>,
    pub play_count: Option<u64>,
    pub played: Option<String>,
    pub starred: Option<String>,
}

/// A playlist's Get Info.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaylistInfo {
    pub id: String,
    pub name: String,
    pub owner: Option<String>,
    pub public: Option<bool>,
    pub comment: Option<String>,
    pub created: Option<String>,
    pub changed: Option<String>,
    pub song_count: Option<u32>,
    pub duration_s: Option<u64>,
    /// Sum of its stored songs' file sizes. `None` until its songs are stored or when none has a size.
    pub size: Option<u64>,
    pub readonly: Option<bool>,
    /// A smart playlist. Navidrome sends `validUntil` only for those.
    pub smart: bool,
    /// Smart only: when the server next re-evaluates its songs.
    pub valid_until: Option<String>,
}

/// An artist's Get Info.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArtistInfo {
    pub id: String,
    pub name: String,
    pub album_count: Option<u32>,
    /// Songs in the library by this artist or on their albums.
    pub song_count: u32,
    /// OpenSubsonic `roles`, in [`ROLE_ORDER`].
    pub roles: Vec<String>,
    pub music_brainz_id: Option<String>,
    /// The signed-in user's plays of those songs.
    pub play_count: u64,
    pub starred: Option<String>,
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned)
}

fn positive(n: Option<i64>) -> Option<u32> {
    n.filter(|n| *n > 0).and_then(|n| u32::try_from(n).ok())
}

fn explicit(status: Option<&str>) -> Option<String> {
    status.filter(|s| matches!(*s, "explicit" | "clean")).map(str::to_owned)
}

/// A list of strings that may arrive as one string; empty ones dropped.
fn strings(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(one)) => vec![one.clone()],
        Some(Value::Array(items)) => items.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
        _ => Vec::new(),
    }
    .into_iter()
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
    .collect()
}

fn role_rank(role: &str) -> usize {
    ROLE_ORDER.iter().position(|r| *r == role).unwrap_or(ROLE_ORDER.len())
}

fn genres_of(list: &[crate::client::NamedRef], single: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = list.iter().map(|g| g.name.trim().to_string()).filter(|g| !g.is_empty()).collect();
    if out.is_empty()
        && let Some(g) = single.map(str::trim).filter(|g| !g.is_empty())
    {
        out.push(g.to_string());
    }
    out
}

fn names(list: &[crate::client::ArtistRef]) -> Vec<CreditName> {
    list.iter().filter(|a| !a.name.is_empty()).map(|a| CreditName { name: a.name.clone(), artist_id: Some(a.id.clone()).filter(|i| !i.is_empty()), sub_role: None }).collect()
}

fn json_err(e: serde_json::Error) -> Error {
    Error::Sync(e.to_string())
}

impl SongInfo {
    /// From a song's JSON as the server sent it (or as the library keeps it).
    pub fn from_json(v: &Value) -> Result<SongInfo> {
        let s: Song = serde_json::from_value(v.clone()).map_err(json_err)?;
        let mut credits: Vec<Credit> = Vec::new();
        for c in &s.contributors {
            if ROLES_SHOWN_ELSEWHERE.contains(&c.role.as_str()) || c.artist.name.is_empty() {
                continue;
            }
            let name = CreditName { name: c.artist.name.clone(), artist_id: Some(c.artist.id.clone()).filter(|i| !i.is_empty()), sub_role: c.sub_role.clone().filter(|r| !r.is_empty()) };
            match credits.iter_mut().find(|g| g.role == c.role) {
                Some(group) if !group.names.contains(&name) => group.names.push(name),
                Some(_) => {}
                None => credits.push(Credit { role: c.role.clone(), names: vec![name] }),
            }
        }
        // Stable: unknown roles keep the server's order.
        credits.sort_by_key(|c| role_rank(&c.role));
        let replay_gain = s.replay_gain.as_ref().map(|g| ReplayGainInfo { track_gain: g.track_gain, album_gain: g.album_gain, track_peak: g.track_peak, album_peak: g.album_peak }).filter(|g| g.track_gain.is_some() || g.album_gain.is_some() || g.track_peak.is_some() || g.album_peak.is_some());
        let artists = names(&s.artists);
        Ok(SongInfo {
            artist: s.artist_line(),
            artists: if artists.len() > 1 { artists } else { Vec::new() },
            album: s.album.clone().filter(|a| !a.is_empty()),
            album_id: s.album_id.clone().filter(|a| !a.is_empty()),
            artist_id: s.artist_id.clone().filter(|a| !a.is_empty()),
            track: positive(s.track.map(i64::from)),
            disc: positive(s.disc_number.map(i64::from)),
            year: s.year.filter(|y| *y > 0),
            genres: genres_of(&s.genres, s.genre.as_deref()),
            duration_s: positive(s.duration),
            size: s.size.filter(|n| *n > 0).map(|n| n as u64),
            suffix: s.suffix.clone().filter(|x| !x.is_empty()),
            content_type: s.content_type.clone().filter(|x| !x.is_empty()),
            bit_rate_kbps: positive(s.bit_rate.map(i64::from)),
            bit_depth: positive(s.bit_depth.map(i64::from)),
            sampling_rate: positive(s.sampling_rate.map(i64::from)),
            channel_count: positive(s.channel_count.map(i64::from)),
            path: s.path.clone().filter(|p| !p.is_empty()),
            bpm: positive(s.bpm.map(i64::from)),
            replay_gain,
            // Read from the JSON so it doesn't depend on the typed model.
            isrc: strings(v.get("isrc")),
            music_brainz_id: s.music_brainz_id.clone().filter(|m| !m.is_empty()),
            comment: text(v, "comment"),
            credits,
            explicit_status: explicit(s.explicit_status.as_deref()),
            created: s.created.clone().filter(|c| !c.is_empty()),
            play_count: s.play_count.map(|n| n.max(0) as u64),
            played: s.played.clone().filter(|p| !p.is_empty()),
            starred: s.starred.clone().filter(|p| !p.is_empty()),
            id: s.id,
            title: s.title,
        })
    }
}

impl AlbumInfo {
    /// From an album's JSON. `totals` are the library's (song count, seconds, bytes, discs).
    pub fn from_json(v: &Value, totals: Option<(u32, u64, Option<u64>, u32)>) -> Result<AlbumInfo> {
        let a: Album = serde_json::from_value(v.clone()).map_err(json_err)?;
        let date = |d: &Option<ItemDate>| d.clone().filter(|d| d.year.is_some_and(|y| y > 0));
        let (songs, seconds, size, discs) = totals.unwrap_or((0, 0, None, 0));
        let disc_count = a.disc_titles.len().max(discs as usize) as u32;
        let artists = names(&a.artists);
        Ok(AlbumInfo {
            artist: a.display_artist.clone().filter(|x| !x.is_empty()).or_else(|| a.artist.clone().filter(|x| !x.is_empty())),
            artist_id: a.artist_id.clone().filter(|x| !x.is_empty()),
            artists: if artists.len() > 1 { artists } else { Vec::new() },
            version: a.version.clone().filter(|x| !x.is_empty()),
            record_labels: a.record_labels.iter().map(|l| l.name.trim().to_string()).filter(|l| !l.is_empty()).collect(),
            release_types: a.release_types.iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect(),
            release_date: date(&a.release_date),
            original_release_date: date(&a.original_release_date),
            year: a.year.filter(|y| *y > 0),
            is_compilation: a.is_compilation,
            music_brainz_id: a.music_brainz_id.clone().filter(|m| !m.is_empty()),
            genres: genres_of(&a.genres, a.genre.as_deref()),
            explicit_status: explicit(a.explicit_status.as_deref()),
            song_count: positive(a.song_count).or(Some(songs).filter(|n| *n > 0)),
            duration_s: a.duration.filter(|d| *d > 0).map(|d| d as u64).or(Some(seconds).filter(|n| *n > 0)),
            disc_count: Some(disc_count).filter(|n| *n > 1),
            size,
            created: a.created.clone().filter(|c| !c.is_empty()),
            play_count: a.play_count.map(|n| n.max(0) as u64),
            played: a.played.clone().filter(|p| !p.is_empty()),
            starred: a.starred.clone().filter(|p| !p.is_empty()),
            id: a.id,
            name: a.name,
        })
    }
}

impl PlaylistInfo {
    /// From a playlist's JSON. `size` is the library's sum over its stored songs.
    pub fn from_json(v: &Value, size: Option<u64>) -> Result<PlaylistInfo> {
        let p: Playlist = serde_json::from_value(v.clone()).map_err(json_err)?;
        Ok(PlaylistInfo {
            owner: p.owner.clone().filter(|x| !x.is_empty()),
            public: p.public,
            comment: p.comment.clone().map(|c| c.trim().to_string()).filter(|c| !c.is_empty()),
            created: p.created.clone().filter(|x| !x.is_empty()),
            changed: p.changed.clone().filter(|x| !x.is_empty()),
            song_count: p.song_count.and_then(|n| u32::try_from(n.max(0)).ok()),
            duration_s: p.duration.filter(|d| *d > 0).map(|d| d as u64),
            size,
            readonly: p.readonly,
            smart: p.valid_until.is_some(),
            valid_until: p.valid_until.clone().filter(|x| !x.is_empty()),
            id: p.id,
            name: p.name,
        })
    }
}

impl ArtistInfo {
    /// From an artist's JSON. `songs` and `plays` are the library's.
    pub fn from_json(v: &Value, songs: u32, plays: u64) -> Result<ArtistInfo> {
        let a: Artist = serde_json::from_value(v.clone()).map_err(json_err)?;
        let mut roles: Vec<String> = a.roles.iter().map(|r| r.trim().to_string()).filter(|r| !r.is_empty()).collect();
        roles.dedup();
        roles.sort_by_key(|r| role_rank(r));
        Ok(ArtistInfo {
            album_count: a.album_count.and_then(|n| u32::try_from(n.max(0)).ok()),
            song_count: songs,
            roles,
            music_brainz_id: a.music_brainz_id.clone().filter(|m| !m.is_empty()),
            play_count: plays,
            starred: a.starred.clone().filter(|s| !s.is_empty()),
            id: a.id,
            name: a.name,
        })
    }
}

fn parse(data: &str) -> Result<Value> {
    serde_json::from_str(data).map_err(json_err)
}

impl Library {
    /// A song's Get Info. Plays and the favorite come from the library's columns, which track
    /// this device's plays between syncs.
    pub fn song_info(&self, id: &str) -> Result<Option<SongInfo>> {
        let row: Option<(String, i64, Option<String>, Option<String>)> = self.read(|c| Ok(c.query_row("SELECT data, play_count, played, starred FROM song WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).optional()?))?;
        let Some((data, plays, played, starred)) = row else { return Ok(None) };
        let mut info = SongInfo::from_json(&parse(&data)?)?;
        info.play_count = Some(plays.max(0) as u64);
        info.played = played.filter(|p| !p.is_empty());
        info.starred = starred.filter(|s| !s.is_empty());
        Ok(Some(info))
    }

    /// An album's Get Info, with totals from its songs in the library.
    pub fn album_info(&self, id: &str) -> Result<Option<AlbumInfo>> {
        self.read(|c| {
            let row: Option<(String, i64, Option<String>, Option<String>)> = c.query_row("SELECT data, play_count, played, starred FROM album WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).optional()?;
            let Some((data, plays, played, starred)) = row else { return Ok(None) };
            let totals: (i64, Option<i64>, Option<i64>, Option<i64>) = c.query_row(
                "SELECT COUNT(*), SUM(duration), SUM(json_extract(data, '$.size')), COUNT(DISTINCT disc) FROM song WHERE album_id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
            let totals = (totals.0 as u32, totals.1.unwrap_or(0).max(0) as u64, totals.2.filter(|n| *n > 0).map(|n| n as u64), totals.3.unwrap_or(0) as u32);
            let mut info = AlbumInfo::from_json(&parse(&data)?, Some(totals))?;
            info.play_count = Some(plays.max(0) as u64);
            info.played = played.filter(|p| !p.is_empty());
            info.starred = starred.filter(|s| !s.is_empty());
            Ok(Some(info))
        })
    }

    /// A playlist's Get Info, with its size from its stored songs.
    pub fn playlist_info(&self, id: &str) -> Result<Option<PlaylistInfo>> {
        self.read(|c| {
            let data: Option<String> = c.query_row("SELECT data FROM playlist WHERE id = ?1", [id], |r| r.get(0)).optional()?;
            let Some(data) = data else { return Ok(None) };
            let size: Option<i64> = c.query_row(
                "SELECT SUM(json_extract(coalesce(s.data, p.data), '$.size')) FROM playlist_song p LEFT JOIN song s ON s.id = p.song_id WHERE p.playlist_id = ?1",
                [id],
                |r| r.get(0),
            )?;
            Ok(Some(PlaylistInfo::from_json(&parse(&data)?, size.filter(|n| *n > 0).map(|n| n as u64))?))
        })
    }

    /// An artist's Get Info, counting the library's songs by them or on their albums.
    pub fn artist_info(&self, id: &str) -> Result<Option<ArtistInfo>> {
        self.read(|c| {
            let row: Option<(String, Option<String>)> = c.query_row("SELECT data, starred FROM artist WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
            let Some((data, starred)) = row else { return Ok(None) };
            let (songs, plays): (i64, Option<i64>) = c.query_row(
                "SELECT COUNT(*), SUM(play_count) FROM song WHERE artist_id = ?1 OR album_id IN (SELECT id FROM album WHERE artist_id = ?1)",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let mut info = ArtistInfo::from_json(&parse(&data)?, songs as u32, plays.unwrap_or(0).max(0) as u64)?;
            info.starred = starred.filter(|s| !s.is_empty());
            Ok(Some(info))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A song as Navidrome 0.64 sends it, trimmed from a real `getSong` answer.
    fn rich_song() -> Value {
        serde_json::from_str(
            r#"{
            "id": "84c0", "parent": "al1", "isDir": false, "title": "Nights", "album": "Blonde", "artist": "Frank Ocean",
            "track": 9, "year": 2016, "genre": "R&B", "coverArt": "mf-84c0", "size": 48213456, "contentType": "audio/flac",
            "suffix": "flac", "duration": 307, "bitRate": 1254, "bitDepth": 24, "samplingRate": 48000, "channelCount": 2,
            "path": "Frank Ocean/Blonde/09 - Nights.flac", "playCount": 12, "played": "2026-11-01T10:00:00Z", "discNumber": 1,
            "created": "2024-03-01T12:00:00Z", "albumId": "al1", "artistId": "ar1", "type": "music", "mediaType": "song",
            "bpm": 90, "comment": "  Remastered  ", "sortName": "nights", "musicBrainzId": "rec-mbid",
            "isrc": ["USUM71612345"],
            "genres": [{"name": "R&B"}, {"name": "Soul"}],
            "artists": [{"id": "ar1", "name": "Frank Ocean"}],
            "displayArtist": "Frank Ocean",
            "albumArtists": [{"id": "ar1", "name": "Frank Ocean"}],
            "contributors": [
                {"role": "artist", "artist": {"id": "ar1", "name": "Frank Ocean"}},
                {"role": "producer", "artist": {"id": "ar2", "name": "Om'Mas Keith"}},
                {"role": "composer", "artist": {"id": "ar1", "name": "Frank Ocean"}},
                {"role": "performer", "subRole": "guitar", "artist": {"id": "ar3", "name": "Alex G"}},
                {"role": "producer", "artist": {"id": "ar4", "name": "Frank Ocean"}},
                {"role": "composer", "artist": {"id": "ar1", "name": "Frank Ocean"}},
                {"role": "albumartist", "artist": {"id": "ar1", "name": "Frank Ocean"}},
                {"role": "whistler", "artist": {"id": "", "name": "Someone"}}
            ],
            "displayComposer": "Frank Ocean", "moods": [], "explicitStatus": "explicit",
            "replayGain": {"trackGain": -9.73, "trackPeak": 0.989, "albumGain": -8.1}
        }"#,
        )
        .unwrap()
    }

    /// A plain Subsonic server: no OpenSubsonic fields at all.
    fn plain_song() -> Value {
        json!({"id": "s1", "title": "Song", "album": "A", "artist": "B", "suffix": "mp3", "duration": 200, "bitRate": 320, "size": 8000000, "contentType": "audio/mpeg", "path": "B/A/Song.mp3", "created": "2020-01-01T00:00:00Z"})
    }

    #[test]
    fn a_rich_song_gives_every_field_and_groups_credits_by_role() {
        let info = SongInfo::from_json(&rich_song()).unwrap();
        assert_eq!((info.size, info.suffix.as_deref(), info.content_type.as_deref()), (Some(48_213_456), Some("flac"), Some("audio/flac")));
        assert_eq!((info.bit_rate_kbps, info.bit_depth, info.sampling_rate, info.channel_count), (Some(1254), Some(24), Some(48_000), Some(2)));
        assert_eq!((info.bpm, info.track, info.disc, info.year), (Some(90), Some(9), Some(1), Some(2016)));
        assert_eq!(info.isrc, ["USUM71612345"]);
        assert_eq!(info.music_brainz_id.as_deref(), Some("rec-mbid"));
        assert_eq!(info.comment.as_deref(), Some("Remastered"));
        assert_eq!(info.explicit_status.as_deref(), Some("explicit"));
        assert_eq!(info.genres, ["R&B", "Soul"]);
        assert_eq!(info.replay_gain, Some(ReplayGainInfo { track_gain: Some(-9.73), album_gain: Some(-8.1), track_peak: Some(0.989), album_peak: None }));
        assert_eq!((info.play_count, info.played.as_deref(), info.created.as_deref()), (Some(12), Some("2026-11-01T10:00:00Z"), Some("2024-03-01T12:00:00Z")));
        assert!(info.artists.is_empty(), "one artist shows in the title area");
        let roles: Vec<&str> = info.credits.iter().map(|c| c.role.as_str()).collect();
        assert_eq!(roles, ["composer", "producer", "performer", "whistler"], "known roles in order, then the server's");
        assert_eq!(info.credits[0].names.len(), 1, "a repeated credit counts once");
        assert_eq!(info.credits[1].names.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(), ["Om'Mas Keith", "Frank Ocean"]);
        assert_eq!(info.credits[2].names[0].sub_role.as_deref(), Some("guitar"));
        assert_eq!(info.credits[3].names[0].artist_id, None);
    }

    #[test]
    fn a_plain_subsonic_song_has_only_what_it_sent() {
        let info = SongInfo::from_json(&plain_song()).unwrap();
        assert_eq!((info.size, info.bit_rate_kbps, info.path.as_deref()), (Some(8_000_000), Some(320), Some("B/A/Song.mp3")));
        assert_eq!((info.bit_depth, info.sampling_rate, info.channel_count, info.bpm), (None, None, None, None));
        assert!(info.isrc.is_empty() && info.credits.is_empty() && info.genres.is_empty());
        assert_eq!((info.replay_gain, info.music_brainz_id, info.comment, info.explicit_status), (None, None, None, None));
        assert_eq!((info.play_count, info.played), (None, None));
        // An ISRC sent as one string, and an empty explicit flag.
        let info = SongInfo::from_json(&json!({"id": "x", "title": "t", "isrc": "GBBKS1700185", "explicitStatus": ""})).unwrap();
        assert_eq!((info.isrc, info.explicit_status), (vec!["GBBKS1700185".to_string()], None));
    }

    #[test]
    fn albums_playlists_and_artists_from_rich_and_plain_json() {
        let rich = json!({
            "id": "al1", "name": "Blonde", "artist": "Frank Ocean", "artistId": "ar1", "songCount": 17, "duration": 3600, "created": "2024-03-01T12:00:00Z",
            "year": 2016, "playCount": 40, "played": "2026-11-01T10:00:00Z", "musicBrainzId": "rel-mbid", "isCompilation": false,
            "recordLabels": [{"name": "Boys Don't Cry"}], "releaseTypes": ["album"], "releaseDate": {"year": 2016, "month": 8, "day": 20},
            "originalReleaseDate": {}, "genres": [{"name": "R&B"}], "explicitStatus": "clean",
            "discTitles": [{"disc": 1, "title": ""}, {"disc": 2, "title": ""}]
        });
        let a = AlbumInfo::from_json(&rich, Some((17, 3600, Some(500_000_000), 1))).unwrap();
        assert_eq!((a.record_labels, a.release_types), (vec!["Boys Don't Cry".to_string()], vec!["album".to_string()]));
        assert_eq!(a.release_date, Some(ItemDate { year: Some(2016), month: Some(8), day: Some(20) }));
        assert_eq!(a.original_release_date, None, "an empty date is no date");
        assert_eq!((a.is_compilation, a.song_count, a.duration_s, a.size, a.disc_count), (Some(false), Some(17), Some(3600), Some(500_000_000), Some(2)));
        assert_eq!(a.explicit_status.as_deref(), Some("clean"));
        let plain = AlbumInfo::from_json(&json!({"id": "al2", "name": "A", "artist": "B"}), Some((3, 600, None, 1))).unwrap();
        assert_eq!((plain.song_count, plain.duration_s, plain.size, plain.disc_count), (Some(3), Some(600), None, None), "totals from the library");
        assert!(plain.record_labels.is_empty() && plain.release_date.is_none() && plain.is_compilation.is_none());

        let smart = PlaylistInfo::from_json(&json!({"id": "p1", "name": "Liked", "owner": "me", "public": false, "songCount": 3, "duration": 600, "created": "2025-01-01T00:00:00Z", "changed": "2026-11-11T00:00:00Z", "readonly": true, "validUntil": "2026-11-11T00:00:05Z", "comment": ""}), Some(1234)).unwrap();
        assert!(smart.smart && smart.readonly == Some(true) && smart.comment.is_none());
        assert_eq!((smart.valid_until.as_deref(), smart.size, smart.song_count), (Some("2026-11-11T00:00:05Z"), Some(1234), Some(3)));
        let regular = PlaylistInfo::from_json(&json!({"id": "p2", "name": "Mix"}), None).unwrap();
        assert!(!regular.smart && regular.owner.is_none() && regular.public.is_none());

        let artist = ArtistInfo::from_json(&json!({"id": "ar1", "name": "Frank Ocean", "albumCount": 3, "musicBrainzId": "art-mbid", "roles": ["producer", "artist", "composer"]}), 40, 99).unwrap();
        assert_eq!(artist.roles, ["artist", "composer", "producer"]);
        assert_eq!((artist.album_count, artist.song_count, artist.play_count, artist.music_brainz_id.as_deref()), (Some(3), 40, 99, Some("art-mbid")));
        let plain = ArtistInfo::from_json(&json!({"id": "ar2", "name": "B"}), 0, 0).unwrap();
        assert!(plain.roles.is_empty() && plain.album_count.is_none() && plain.music_brainz_id.is_none());
    }

    #[test]
    fn library_rows_give_info_with_derived_totals() {
        use crate::client::MusicFolder;
        use crate::library::LibraryConfig;
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(&LibraryConfig { path: dir.path().join("lib.sqlite") }).unwrap();
        lib.replace_libraries(&[MusicFolder { id: "1".into(), name: "L".into() }]).unwrap();
        let cur = lib.begin_sync("1", None, false).unwrap();
        lib.apply_albums("1", &cur, 1, &[Album { id: "al1".into(), name: "Blonde".into(), artist_id: Some("ar1".into()), ..Default::default() }]).unwrap();
        lib.apply_artists("1", &cur, 1, &[Artist { id: "ar1".into(), name: "Frank Ocean".into(), album_count: Some(1), ..Default::default() }]).unwrap();
        let song = |id: &str, size: i64, disc: i32, plays: i64| Song { id: id.into(), title: id.into(), album_id: Some("al1".into()), artist_id: Some("ar9".into()), size: Some(size), duration: Some(100), disc_number: Some(disc), play_count: Some(plays), ..Default::default() };
        lib.apply_songs("1", &cur, 2, &[song("s1", 1000, 1, 2), song("s2", 3000, 2, 5)]).unwrap();
        // The stored JSON as a newer sync keeps it, with the ISRC.
        let mut rich = rich_song();
        rich["id"] = json!("s1");
        lib.write(|tx| {
            tx.execute("UPDATE song SET data = ?1 WHERE id = 's1'", [rich.to_string()])?;
            Ok(())
        })
        .unwrap();
        let s = lib.song_info("s1").unwrap().unwrap();
        assert_eq!((s.isrc.len(), s.play_count), (1, Some(2)), "plays from the row");
        let a = lib.album_info("al1").unwrap().unwrap();
        assert_eq!((a.song_count, a.duration_s, a.disc_count), (Some(2), Some(200), Some(2)));
        assert_eq!(a.size, Some(48_213_456 + 3000), "sizes summed from the stored songs");
        let r = lib.artist_info("ar1").unwrap().unwrap();
        assert_eq!((r.song_count, r.play_count), (2, 7), "songs on their albums count");
        assert!(lib.song_info("nope").unwrap().is_none() && lib.playlist_info("nope").unwrap().is_none());
    }
}
