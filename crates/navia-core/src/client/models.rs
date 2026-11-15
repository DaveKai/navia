//! Typed OpenSubsonic responses. Fields are lenient so that one missing field never fails a
//! response, and Navidrome omits empty lists entirely.

use serde::de::{DeserializeOwned, Deserializer};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

/// Deserialize a list, logging and dropping any item that doesn't parse.
pub(crate) fn lenient_vec<'de, D, T>(d: D) -> std::result::Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let values: Option<Vec<serde_json::Value>> = Option::deserialize(d)?;
    Ok(values
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| match serde_json::from_value::<T>(v) {
            Ok(item) => Some(item),
            Err(e) => {
                tracing::warn!("skipping list item that failed to parse: {e}");
                None
            }
        })
        .collect())
}

/// Accept a JSON string or integer. Navidrome's docs say IDs are strings, but `musicFolder.id`
/// is a JSON integer in practice.
pub(crate) fn string_or_int<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<String, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Id {
        S(String),
        I(i64),
    }
    Ok(match Id::deserialize(d)? {
        Id::S(s) => s,
        Id::I(i) => i.to_string(),
    })
}

/// Identifies the server, from any response envelope.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    pub api_version: String,
    /// Server product name, e.g. `navidrome`. Absent on servers without OpenSubsonic.
    pub kind: Option<String>,
    pub server_version: Option<String>,
    pub open_subsonic: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Extension {
    pub name: String,
    pub versions: Vec<i32>,
}

/// One library, as exposed to this user by `getMusicFolders`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MusicFolder {
    #[serde(deserialize_with = "string_or_int")]
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScanStatus {
    pub scanning: bool,
    pub count: Option<u64>,
    pub folder_count: Option<u64>,
    /// Navidrome extra: when the last scan ended. `None` on servers that don't report it.
    pub last_scan: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ReplayGain {
    pub track_gain: Option<f64>,
    pub album_gain: Option<f64>,
    pub track_peak: Option<f64>,
    pub album_peak: Option<f64>,
    pub base_gain: Option<f64>,
    pub fallback_gain: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NamedRef {
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ArtistRef {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Contributor {
    pub role: String,
    pub sub_role: Option<String>,
    pub artist: ArtistRef,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ItemDate {
    pub year: Option<i32>,
    pub month: Option<i32>,
    pub day: Option<i32>,
}

/// A song, as returned everywhere the API returns a song.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Song {
    pub id: String,
    pub parent: Option<String>,
    pub title: String,
    pub album: Option<String>,
    pub artist: Option<String>,
    pub album_id: Option<String>,
    pub artist_id: Option<String>,
    pub track: Option<i32>,
    pub disc_number: Option<i32>,
    pub year: Option<i32>,
    pub genre: Option<String>,
    pub cover_art: Option<String>,
    pub size: Option<i64>,
    pub content_type: Option<String>,
    pub suffix: Option<String>,
    pub duration: Option<i64>,
    pub bit_rate: Option<i32>,
    pub bit_depth: Option<i32>,
    pub sampling_rate: Option<i32>,
    pub channel_count: Option<i32>,
    pub path: Option<String>,
    pub created: Option<String>,
    pub starred: Option<String>,
    pub played: Option<String>,
    pub play_count: Option<i64>,
    pub user_rating: Option<i32>,
    pub bpm: Option<i32>,
    pub comment: Option<String>,
    pub sort_name: Option<String>,
    pub music_brainz_id: Option<String>,
    #[serde(deserialize_with = "lenient_vec", skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<NamedRef>,
    #[serde(deserialize_with = "lenient_vec", skip_serializing_if = "Vec::is_empty")]
    pub moods: Vec<String>,
    pub replay_gain: Option<ReplayGain>,
    #[serde(deserialize_with = "lenient_vec", skip_serializing_if = "Vec::is_empty")]
    pub artists: Vec<ArtistRef>,
    pub display_artist: Option<String>,
    #[serde(deserialize_with = "lenient_vec", skip_serializing_if = "Vec::is_empty")]
    pub album_artists: Vec<ArtistRef>,
    pub display_album_artist: Option<String>,
    #[serde(deserialize_with = "lenient_vec", skip_serializing_if = "Vec::is_empty")]
    pub contributors: Vec<Contributor>,
    pub display_composer: Option<String>,
    pub explicit_status: Option<String>,
    /// OpenSubsonic: the recording's ISRCs.
    #[serde(deserialize_with = "strings_lenient", skip_serializing_if = "Vec::is_empty")]
    pub isrc: Vec<String>,
}

/// A list of strings that may arrive as one string, or with items of other types (dropped).
fn strings_lenient<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Vec<String>, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::String(one) => vec![one],
        serde_json::Value::Array(items) => items.into_iter().filter_map(|v| v.as_str().map(str::to_owned)).collect(),
        _ => Vec::new(),
    }
    .into_iter()
    .filter(|s| !s.trim().is_empty())
    .collect())
}

impl Song {
    /// Every credited artist joined with ", ", else `displayArtist`, else `artist`.
    pub fn artist_line(&self) -> Option<String> {
        let names: Vec<&str> = self.artists.iter().map(|a| a.name.trim()).filter(|n| !n.is_empty()).collect();
        if !names.is_empty() {
            return Some(names.join(", "));
        }
        self.display_artist.clone().filter(|a| !a.trim().is_empty()).or_else(|| self.artist.clone().filter(|a| !a.trim().is_empty()))
    }

    /// Each credited artist's name: OpenSubsonic `artists`, else the one artist text.
    pub fn artist_names(&self) -> Vec<String> {
        let names: Vec<String> = self.artists.iter().map(|a| a.name.trim()).filter(|n| !n.is_empty()).map(str::to_owned).collect();
        if !names.is_empty() {
            return names;
        }
        self.artist_line().into_iter().collect()
    }

    /// Every genre of the song: OpenSubsonic `genres`, else its one `genre`.
    pub fn genre_names(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for g in &self.genres {
            let name = g.name.trim();
            if !name.is_empty() && !names.iter().any(|n| n == name) {
                names.push(name.to_owned());
            }
        }
        if names.is_empty()
            && let Some(one) = self.genre.as_deref().map(str::trim).filter(|g| !g.is_empty())
        {
            names.push(one.to_owned());
        }
        names
    }

    /// The explicit status as Navidrome's smart playlist rules store it, "e" or "c".
    /// The API sends the words, the rules compare the letters.
    pub fn explicit_code(&self) -> Option<&'static str> {
        match self.explicit_status.as_deref().map(str::trim) {
            Some(s) if s.eq_ignore_ascii_case("explicit") || s.eq_ignore_ascii_case("e") => Some("e"),
            Some(s) if s.eq_ignore_ascii_case("clean") || s.eq_ignore_ascii_case("c") => Some("c"),
            _ => None,
        }
    }

    /// The album artist as shown: `displayAlbumArtist`, else the `albumArtists` joined.
    pub fn album_artist_line(&self) -> Option<String> {
        if let Some(a) = self.display_album_artist.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
            return Some(a.to_owned());
        }
        let names: Vec<&str> = self.album_artists.iter().map(|a| a.name.trim()).filter(|n| !n.is_empty()).collect();
        (!names.is_empty()).then(|| names.join(", "))
    }
}

/// An album, as a list entry (no `song` list) or with its songs from `getAlbum`.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Album {
    pub id: String,
    pub name: String,
    pub artist: Option<String>,
    pub artist_id: Option<String>,
    pub cover_art: Option<String>,
    pub song_count: Option<i64>,
    pub duration: Option<i64>,
    pub created: Option<String>,
    pub year: Option<i32>,
    pub genre: Option<String>,
    pub starred: Option<String>,
    pub played: Option<String>,
    pub play_count: Option<i64>,
    pub user_rating: Option<i32>,
    #[serde(deserialize_with = "lenient_vec", skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<NamedRef>,
    pub music_brainz_id: Option<String>,
    pub is_compilation: Option<bool>,
    pub sort_name: Option<String>,
    pub original_release_date: Option<ItemDate>,
    pub release_date: Option<ItemDate>,
    #[serde(deserialize_with = "lenient_vec", skip_serializing_if = "Vec::is_empty")]
    pub release_types: Vec<String>,
    #[serde(deserialize_with = "lenient_vec", skip_serializing_if = "Vec::is_empty")]
    pub record_labels: Vec<NamedRef>,
    #[serde(deserialize_with = "lenient_vec", skip_serializing_if = "Vec::is_empty")]
    pub moods: Vec<String>,
    #[serde(deserialize_with = "lenient_vec", skip_serializing_if = "Vec::is_empty")]
    pub artists: Vec<ArtistRef>,
    pub display_artist: Option<String>,
    pub explicit_status: Option<String>,
    pub version: Option<String>,
    /// OpenSubsonic: each disc's number and subtitle, which can be empty.
    #[serde(deserialize_with = "lenient_vec", skip_serializing_if = "Vec::is_empty")]
    pub disc_titles: Vec<DiscTitle>,
    /// Present only from `getAlbum`.
    #[serde(deserialize_with = "lenient_vec", skip_serializing_if = "Vec::is_empty")]
    pub song: Vec<Song>,
}

/// One disc of an album, from OpenSubsonic `discTitles`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscTitle {
    pub disc: i32,
    pub title: String,
}

/// Whether OpenSubsonic's `explicitStatus` marks an item explicit. "clean" and empty don't.
pub fn is_explicit(status: Option<&str>) -> bool {
    status.is_some_and(|s| s.eq_ignore_ascii_case("explicit"))
}

#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Artist {
    pub id: String,
    pub name: String,
    pub cover_art: Option<String>,
    pub artist_image_url: Option<String>,
    pub album_count: Option<i64>,
    pub starred: Option<String>,
    pub user_rating: Option<i32>,
    pub music_brainz_id: Option<String>,
    pub sort_name: Option<String>,
    #[serde(deserialize_with = "lenient_vec", skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
    /// Present only from `getArtist`.
    #[serde(deserialize_with = "lenient_vec", skip_serializing_if = "Vec::is_empty")]
    pub album: Vec<Album>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ArtistIndex {
    pub name: String,
    #[serde(deserialize_with = "lenient_vec")]
    pub artist: Vec<Artist>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Artists {
    pub ignored_articles: Option<String>,
    pub last_modified: Option<i64>,
    #[serde(deserialize_with = "lenient_vec")]
    pub index: Vec<ArtistIndex>,
}

/// `getArtistInfo2`. Navidrome needs external integrations for most of this, so expect blanks.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ArtistInfo {
    pub biography: Option<String>,
    pub music_brainz_id: Option<String>,
    pub last_fm_url: Option<String>,
    pub small_image_url: Option<String>,
    pub medium_image_url: Option<String>,
    pub large_image_url: Option<String>,
    #[serde(deserialize_with = "lenient_vec")]
    pub similar_artist: Vec<Artist>,
}

/// Playlist summary or, from `getPlaylist`, with `entry` populated.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub comment: Option<String>,
    pub owner: Option<String>,
    pub public: Option<bool>,
    pub song_count: Option<i64>,
    pub duration: Option<i64>,
    pub created: Option<String>,
    pub changed: Option<String>,
    pub cover_art: Option<String>,
    /// Navidrome extra: true for smart playlists and others this user can't change.
    pub readonly: Option<bool>,
    /// OpenSubsonic: when a smart playlist's contents need re-evaluating. Only smart playlists have it.
    pub valid_until: Option<String>,
    #[serde(deserialize_with = "lenient_vec")]
    pub entry: Vec<Song>,
}

/// A change to a playlist's details through `updatePlaylist`: only the fields that are `Some`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlaylistDetails {
    pub name: Option<String>,
    pub comment: Option<String>,
    pub public: Option<bool>,
}

/// Everything this user has starred (`getStarred2`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Starred {
    #[serde(deserialize_with = "lenient_vec")]
    pub artist: Vec<Artist>,
    #[serde(deserialize_with = "lenient_vec")]
    pub album: Vec<Album>,
    #[serde(deserialize_with = "lenient_vec")]
    pub song: Vec<Song>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LyricLine {
    pub start: Option<i64>,
    /// The exact end, only when the source has one (`songLyrics` v2, `enhanced=true`).
    pub end: Option<i64>,
    pub value: String,
}

/// Word or syllable timing for one line (`songLyrics` v2, `enhanced=true`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CueLine {
    /// The `line` this belongs to. Several cue lines can share one, one per vocal agent.
    pub index: Option<i64>,
    pub start: Option<i64>,
    pub end: Option<i64>,
    pub value: Option<String>,
    pub agent_id: Option<String>,
    #[serde(deserialize_with = "lenient_vec")]
    pub cue: Vec<Cue>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Cue {
    pub start: Option<i64>,
    pub end: Option<i64>,
    pub value: String,
    /// Inclusive UTF-8 byte offsets into the cue line's `value`.
    pub byte_start: Option<i64>,
    pub byte_end: Option<i64>,
}

/// A vocal part named by cue lines (`songLyrics` v2).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LyricAgent {
    pub id: String,
    pub role: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StructuredLyrics {
    pub display_artist: Option<String>,
    pub display_title: Option<String>,
    pub lang: Option<String>,
    pub offset: Option<i64>,
    pub synced: Option<bool>,
    #[serde(deserialize_with = "lenient_vec")]
    pub line: Vec<LyricLine>,
    /// `main`, `translation` or `pronunciation` (`songLyrics` v2). Absent means main.
    pub kind: Option<String>,
    #[serde(deserialize_with = "lenient_vec")]
    pub agents: Vec<LyricAgent>,
    #[serde(deserialize_with = "lenient_vec")]
    pub cue_line: Vec<CueLine>,
}

/// `getAlbumInfo2`: notes and links from external sources, when the server has them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AlbumInfo {
    pub notes: Option<String>,
    pub music_brainz_id: Option<String>,
    pub last_fm_url: Option<String>,
    /// Navidrome: a public, signed `/share/img/<token>` link on the address the request used.
    pub large_image_url: Option<String>,
}

/// Classic `getLyrics`, unsynced and matched by artist and title.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlainLyrics {
    pub artist: Option<String>,
    pub title: Option<String>,
    pub value: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InternetRadioStation {
    pub id: String,
    pub name: String,
    pub stream_url: Option<String>,
    pub home_page_url: Option<String>,
    pub cover_art: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlayQueue {
    /// Navidrome documents this as a string ID, not the integer of older Subsonic servers.
    pub current: Option<String>,
    pub position: Option<i64>,
    pub username: Option<String>,
    pub changed: Option<String>,
    pub changed_by: Option<String>,
    #[serde(deserialize_with = "lenient_vec")]
    pub entry: Vec<Song>,
}

/// `getPlayQueueByIndex` (extension `indexBasedQueue`): the current song by position, so a
/// queue can hold the same song twice.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlayQueueByIndex {
    pub current_index: Option<i64>,
    pub position: Option<i64>,
    pub username: Option<String>,
    pub changed: Option<String>,
    pub changed_by: Option<String>,
    #[serde(deserialize_with = "lenient_vec")]
    pub entry: Vec<Song>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Genre {
    pub value: String,
    pub song_count: Option<i64>,
    pub album_count: Option<i64>,
}

/// The authenticated user's own account roles.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct User {
    pub username: String,
    pub scrobbling_enabled: Option<bool>,
    pub admin_role: Option<bool>,
    pub settings_role: Option<bool>,
    pub stream_role: Option<bool>,
    pub download_role: Option<bool>,
    pub share_role: Option<bool>,
    pub playlist_role: Option<bool>,
    pub cover_art_role: Option<bool>,
    pub comment_role: Option<bool>,
    pub podcast_role: Option<bool>,
    pub jukebox_role: Option<bool>,
    #[serde(deserialize_with = "lenient_vec")]
    pub folder: Vec<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Share {
    pub id: String,
    pub url: Option<String>,
    pub description: Option<String>,
    pub username: Option<String>,
    pub created: Option<String>,
    pub expires: Option<String>,
    pub last_visited: Option<String>,
    pub visit_count: Option<i64>,
    /// What it shares: albums for album and artist shares (`isDir`), songs otherwise.
    #[serde(deserialize_with = "lenient_vec")]
    pub entry: Vec<ShareEntry>,
}

/// One item of a share's `entry`: an album (`is_dir`) or a song.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ShareEntry {
    pub id: String,
    pub is_dir: Option<bool>,
    pub title: Option<String>,
    pub album: Option<String>,
    pub artist: Option<String>,
    pub album_id: Option<String>,
    pub artist_id: Option<String>,
    pub cover_art: Option<String>,
}

/// One page of a full-library scan via `search3`.
#[derive(Clone, Debug, Default)]
pub struct SearchPage<T> {
    pub items: Vec<T>,
    /// Items the server sent, including any we couldn't parse. Paging offsets use this.
    pub raw_count: usize,
}

/// Options for `stream` URLs.
#[derive(Clone, Debug, Default)]
pub struct StreamOptions {
    pub max_bit_rate: Option<u32>,
    /// Target format, e.g. `opus`. `raw` asks for the original file.
    pub format: Option<String>,
    pub time_offset: Option<u32>,
}

/// Parameters for `getAlbumList2`. The server caps `size` at 500 regardless of the request.
#[derive(Clone, Debug)]
pub struct AlbumListOptions {
    pub list_type: String,
    pub size: u32,
    pub offset: u32,
    pub music_folder_id: Option<String>,
    pub from_year: Option<i32>,
    pub to_year: Option<i32>,
    pub genre: Option<String>,
}

impl AlbumListOptions {
    pub fn new(list_type: &str, size: u32, offset: u32) -> Self {
        Self {
            list_type: list_type.to_string(),
            size,
            offset,
            music_folder_id: None,
            from_year: None,
            to_year: None,
            genre: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isrc_takes_a_list_a_string_or_nothing() {
        let song = |isrc: serde_json::Value| serde_json::from_value::<Song>(serde_json::json!({"id": "1", "title": "t", "isrc": isrc})).unwrap().isrc;
        assert_eq!(song(serde_json::json!(["QZAAA2600001", 3, ""])), vec!["QZAAA2600001"]);
        assert_eq!(song(serde_json::json!("QZAAA1700002")), vec!["QZAAA1700002"]);
        assert!(song(serde_json::json!(null)).is_empty());
        assert!(serde_json::from_value::<Song>(serde_json::json!({"id": "1", "title": "t"})).unwrap().isrc.is_empty());
    }

    #[test]
    fn artist_line_names_every_artist_then_falls_back() {
        let song = |v: serde_json::Value| serde_json::from_value::<Song>(v).unwrap().artist_line();
        let both = serde_json::json!({"id": "1", "title": "death bed", "artist": "Powfu", "displayArtist": "Powfu",
            "artists": [{"id": "a", "name": "Powfu"}, {"id": "b", "name": "beabadoobee"}]});
        assert_eq!(song(both).as_deref(), Some("Powfu, beabadoobee"));
        assert_eq!(song(serde_json::json!({"id": "1", "title": "t", "artist": "A", "displayArtist": "A & B"})).as_deref(), Some("A & B"));
        assert_eq!(song(serde_json::json!({"id": "1", "title": "t", "artist": "A", "displayArtist": ""})).as_deref(), Some("A"));
        assert_eq!(song(serde_json::json!({"id": "1", "title": "t"})), None);
    }
}
