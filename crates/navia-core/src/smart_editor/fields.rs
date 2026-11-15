//! Navidrome's smart playlist vocabulary, read from its v0.64.2 source.
//! Ids are its exact field and operator names, and labels are English catalog keys.

use std::collections::HashMap;
use std::sync::LazyLock;

use super::n_;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FieldKind {
    Text,
    Tag,
    Genre,
    Number,
    Year,
    Date,
    Bool,
    Playlist,
}

#[derive(Clone, Copy, Debug)]
pub struct Field {
    pub id: &'static str,
    pub label: &'static str,
    pub kind: FieldKind,
    pub group: &'static str,
    pub nullable: bool,
    pub unit: Option<&'static str>,
    pub hint: Option<&'static str>,
}

const fn f(id: &'static str, label: &'static str, kind: FieldKind, group: &'static str, nullable: bool) -> Field {
    Field { id, label, kind, group, nullable, unit: None, hint: None }
}
const fn u(id: &'static str, label: &'static str, kind: FieldKind, group: &'static str, nullable: bool, unit: &'static str) -> Field {
    Field { id, label, kind, group, nullable, unit: Some(unit), hint: None }
}
const fn h(id: &'static str, label: &'static str, kind: FieldKind, group: &'static str, nullable: bool, hint: &'static str) -> Field {
    Field { id, label, kind, group, nullable, unit: None, hint: Some(hint) }
}

use FieldKind::*;

/// Every field offered. Mood is a real tag and is left out on purpose.
pub const FIELDS: &[Field] = &[
    f("title", n_("Title"), Text, n_("Song"), false),
    f("album", n_("Album"), Text, n_("Song"), true),
    f("discsubtitle", n_("Disc Subtitle"), Text, n_("Song"), true),
    f("comment", n_("Comment"), Text, n_("Song"), true),
    f("lyrics", n_("Lyrics"), Text, n_("Song"), true),
    f("albumcomment", n_("Album Comment"), Text, n_("Song"), true),
    f("catalognumber", n_("Catalog Number"), Text, n_("Song"), true),
    h("explicitstatus", n_("Explicit Status"), Text, n_("Song"), true, n_("e or c")),
    f("genre", n_("Genre"), Genre, n_("Song"), true),
    f("tracknumber", n_("Track Number"), Number, n_("Song"), false),
    f("discnumber", n_("Disc Number"), Number, n_("Song"), false),
    f("compilation", n_("Compilation"), Bool, n_("Song"), false),
    f("year", n_("Year"), Year, n_("Release"), false),
    f("originalyear", n_("Original Year"), Year, n_("Release"), false),
    f("releaseyear", n_("Release Year"), Year, n_("Release"), false),
    f("date", n_("Recording Date"), Date, n_("Release"), false),
    f("originaldate", n_("Original Date"), Date, n_("Release"), false),
    f("releasedate", n_("Release Date"), Date, n_("Release"), false),
    f("artist", n_("Artist"), Tag, n_("People"), false),
    f("albumartist", n_("Album Artist"), Tag, n_("People"), false),
    f("maincredit", n_("Main Credit"), Tag, n_("People"), false),
    f("composer", n_("Composer"), Tag, n_("People"), false),
    f("lyricist", n_("Lyricist"), Tag, n_("People"), false),
    f("arranger", n_("Arranger"), Tag, n_("People"), false),
    f("conductor", n_("Conductor"), Tag, n_("People"), false),
    f("director", n_("Director"), Tag, n_("People"), false),
    f("producer", n_("Producer"), Tag, n_("People"), false),
    f("engineer", n_("Engineer"), Tag, n_("People"), false),
    f("mixer", n_("Mixer"), Tag, n_("People"), false),
    f("remixer", n_("Remixer"), Tag, n_("People"), false),
    f("djmixer", n_("DJ Mixer"), Tag, n_("People"), false),
    f("performer", n_("Performer"), Tag, n_("People"), false),
    f("playcount", n_("Play Count"), Number, n_("Listening"), false),
    f("lastplayed", n_("Last Played"), Date, n_("Listening"), false),
    f("dateloved", n_("Date Favorited"), Date, n_("Listening"), false),
    f("loved", n_("Favorite"), Bool, n_("Listening"), false),
    h("rating", n_("Rating"), Number, n_("Ratings"), false, n_("0 to 5")),
    h("averagerating", n_("Average Rating"), Number, n_("Ratings"), false, n_("0 to 5")),
    h("albumrating", n_("Album Rating"), Number, n_("Ratings"), false, n_("0 to 5")),
    h("artistrating", n_("Artist Rating"), Number, n_("Ratings"), false, n_("0 to 5")),
    f("daterated", n_("Date Rated"), Date, n_("Ratings"), false),
    f("albumdaterated", n_("Album Date Rated"), Date, n_("Ratings"), false),
    f("artistdaterated", n_("Artist Date Rated"), Date, n_("Ratings"), false),
    f("albumplaycount", n_("Album Play Count"), Number, n_("Album"), false),
    u("albumduration", n_("Album Length"), Number, n_("Album"), false, n_("sec")),
    f("albumsongcount", n_("Album Song Count"), Number, n_("Album"), false),
    u("albumsize", n_("Album Size"), Number, n_("Album"), false, n_("bytes")),
    f("albumlastplayed", n_("Album Last Played"), Date, n_("Album"), false),
    f("albumdateloved", n_("Album Date Favorited"), Date, n_("Album"), false),
    f("albumdateadded", n_("Album Date Added"), Date, n_("Album"), false),
    f("albumdatemodified", n_("Album Date Modified"), Date, n_("Album"), false),
    f("albumloved", n_("Album Favorite"), Bool, n_("Album"), false),
    f("artistplaycount", n_("Artist Play Count"), Number, n_("Artist"), false),
    f("artistlastplayed", n_("Artist Last Played"), Date, n_("Artist"), false),
    f("artistdateloved", n_("Artist Date Favorited"), Date, n_("Artist"), false),
    f("artistloved", n_("Artist Favorite"), Bool, n_("Artist"), false),
    f("dateadded", n_("Date Added"), Date, n_("File"), false),
    f("datemodified", n_("Date Modified"), Date, n_("File"), false),
    u("duration", n_("Length"), Number, n_("File"), false, n_("sec")),
    u("bitrate", n_("Bitrate"), Number, n_("File"), false, n_("kbps")),
    u("bitdepth", n_("Bit Depth"), Number, n_("File"), true, n_("bit")),
    u("samplerate", n_("Sample Rate"), Number, n_("File"), false, n_("Hz")),
    f("bpm", n_("BPM"), Number, n_("File"), true),
    f("channels", n_("Channels"), Number, n_("File"), false),
    u("size", n_("File Size"), Number, n_("File"), false, n_("bytes")),
    f("library_id", n_("Library ID"), Number, n_("File"), false),
    h("filetype", n_("File Type"), Text, n_("File"), false, "flac"),
    h("codec", n_("Codec"), Text, n_("File"), false, n_("FLAC")),
    f("filepath", n_("File Path"), Text, n_("File"), false),
    f("hascoverart", n_("Has Cover Art"), Bool, n_("File"), false),
    f("missing", n_("Missing File"), Bool, n_("File"), false),
    f("id", n_("Playlist"), Playlist, n_("Playlists"), false),
    f("sorttitle", n_("Sort Title"), Text, n_("Sort Names"), true),
    f("sortalbum", n_("Sort Album"), Text, n_("Sort Names"), true),
    f("sortartist", n_("Sort Artist"), Text, n_("Sort Names"), true),
    f("sortalbumartist", n_("Sort Album Artist"), Text, n_("Sort Names"), true),
    u("rgtrackgain", n_("Track Gain"), Number, n_("ReplayGain"), true, "dB"),
    f("rgtrackpeak", n_("Track Peak"), Number, n_("ReplayGain"), true),
    u("rgalbumgain", n_("Album Gain"), Number, n_("ReplayGain"), true, "dB"),
    f("rgalbumpeak", n_("Album Peak"), Number, n_("ReplayGain"), true),
    f("mbz_recording_id", n_("Recording ID"), Text, n_("MusicBrainz IDs"), true),
    f("mbz_album_id", n_("Album ID"), Text, n_("MusicBrainz IDs"), true),
    f("mbz_artist_id", n_("Artist ID"), Text, n_("MusicBrainz IDs"), true),
    f("mbz_album_artist_id", n_("Album Artist ID"), Text, n_("MusicBrainz IDs"), true),
    f("mbz_release_track_id", n_("Release Track ID"), Text, n_("MusicBrainz IDs"), true),
    f("mbz_release_group_id", n_("Release Group ID"), Text, n_("MusicBrainz IDs"), true),
    f("recordlabel", n_("Record Label"), Tag, n_("Tags"), false),
    f("releasetype", n_("Release Type"), Tag, n_("Tags"), false),
    f("releasestatus", n_("Release Status"), Tag, n_("Tags"), false),
    f("releasecountry", n_("Release Country"), Tag, n_("Tags"), false),
    f("albumversion", n_("Album Version"), Tag, n_("Tags"), false),
    f("media", n_("Media"), Tag, n_("Tags"), false),
    f("grouping", n_("Grouping"), Tag, n_("Tags"), false),
    f("language", n_("Language"), Tag, n_("Tags"), false),
    f("script", n_("Script"), Tag, n_("Tags"), false),
    f("key", n_("Musical Key"), Tag, n_("Tags"), false),
    f("subtitle", n_("Subtitle"), Tag, n_("Tags"), false),
    f("work", n_("Work"), Tag, n_("Tags"), false),
    f("movement", n_("Movement"), Tag, n_("Tags"), false),
    f("movementname", n_("Movement Name"), Tag, n_("Tags"), false),
    f("movementtotal", n_("Movement Total"), Tag, n_("Tags"), false),
    f("track", n_("Track Tag"), Tag, n_("Tags"), false),
    f("tracktotal", n_("Track Total"), Tag, n_("Tags"), false),
    f("disc", n_("Disc Tag"), Tag, n_("Tags"), false),
    f("disctotal", n_("Disc Total"), Tag, n_("Tags"), false),
    f("artists", n_("Artists Tag"), Tag, n_("Tags"), false),
    f("albumartists", n_("Album Artists Tag"), Tag, n_("Tags"), false),
    f("titlesort", n_("Title Sort Tag"), Tag, n_("Tags"), false),
    f("albumsort", n_("Album Sort Tag"), Tag, n_("Tags"), false),
    f("artistsort", n_("Artist Sort Tag"), Tag, n_("Tags"), false),
    f("artistssort", n_("Artists Sort Tag"), Tag, n_("Tags"), false),
    f("albumartistsort", n_("Album Artist Sort Tag"), Tag, n_("Tags"), false),
    f("albumartistssort", n_("Album Artists Sort Tag"), Tag, n_("Tags"), false),
    f("composersort", n_("Composer Sort Tag"), Tag, n_("Tags"), false),
    f("lyricistsort", n_("Lyricist Sort Tag"), Tag, n_("Tags"), false),
    f("isrc", n_("ISRC"), Tag, n_("Tags"), false),
    f("barcode", n_("Barcode"), Tag, n_("Tags"), false),
    f("asin", n_("ASIN"), Tag, n_("Tags"), false),
    f("copyright", n_("Copyright"), Tag, n_("Tags"), false),
    f("license", n_("License"), Tag, n_("Tags"), false),
    f("website", n_("Website"), Tag, n_("Tags"), false),
    f("encodedby", n_("Encoded By"), Tag, n_("Tags"), false),
    f("encodersettings", n_("Encoder Settings"), Tag, n_("Tags"), false),
    f("r128_track_gain", n_("R128 Track Gain"), Tag, n_("Tags"), false),
    f("r128_album_gain", n_("R128 Album Gain"), Tag, n_("Tags"), false),
    f("musicbrainz_artistid", n_("MusicBrainz Artist Tag"), Tag, n_("Tags"), false),
    f("musicbrainz_albumartistid", n_("MusicBrainz Album Artist Tag"), Tag, n_("Tags"), false),
    f("musicbrainz_albumid", n_("MusicBrainz Album Tag"), Tag, n_("Tags"), false),
    f("musicbrainz_recordingid", n_("MusicBrainz Recording Tag"), Tag, n_("Tags"), false),
    f("musicbrainz_trackid", n_("MusicBrainz Track Tag"), Tag, n_("Tags"), false),
    f("musicbrainz_releasegroupid", n_("MusicBrainz Release Group Tag"), Tag, n_("Tags"), false),
    f("musicbrainz_workid", n_("MusicBrainz Work Tag"), Tag, n_("Tags"), false),
    f("musicbrainz_discid", n_("MusicBrainz Disc Tag"), Tag, n_("Tags"), false),
    f("musicbrainz_composerid", n_("MusicBrainz Composer Tag"), Tag, n_("Tags"), false),
    f("musicbrainz_lyricistid", n_("MusicBrainz Lyricist Tag"), Tag, n_("Tags"), false),
    f("musicbrainz_directorid", n_("MusicBrainz Director Tag"), Tag, n_("Tags"), false),
    f("musicbrainz_producerid", n_("MusicBrainz Producer Tag"), Tag, n_("Tags"), false),
    f("musicbrainz_engineerid", n_("MusicBrainz Engineer Tag"), Tag, n_("Tags"), false),
    f("musicbrainz_mixerid", n_("MusicBrainz Mixer Tag"), Tag, n_("Tags"), false),
    f("musicbrainz_remixerid", n_("MusicBrainz Remixer Tag"), Tag, n_("Tags"), false),
    f("musicbrainz_djmixerid", n_("MusicBrainz DJ Mixer Tag"), Tag, n_("Tags"), false),
    f("musicbrainz_conductorid", n_("MusicBrainz Conductor Tag"), Tag, n_("Tags"), false),
    f("musicbrainz_arrangerid", n_("MusicBrainz Arranger Tag"), Tag, n_("Tags"), false),
    f("musicbrainz_performerid", n_("MusicBrainz Performer Tag"), Tag, n_("Tags"), false),
];

static FIELD_BY_ID: LazyLock<HashMap<&'static str, &'static Field>> = LazyLock::new(|| FIELDS.iter().map(|f| (f.id, f)).collect());

pub fn field_by_id(id: &str) -> Option<&'static Field> {
    FIELD_BY_ID.get(id).copied()
}

/// Older names Navidrome accepts, and the field each one means.
pub(crate) const FIELD_ALIASES: &[(&str, &str)] = &[("albumtype", "releasetype"), ("recordingdate", "date"), ("replaygain_album_gain", "rgalbumgain"), ("replaygain_album_peak", "rgalbumpeak"), ("replaygain_track_gain", "rgtrackgain"), ("replaygain_track_peak", "rgtrackpeak")];

const OP_LABELS: &[(&str, &str)] = &[
    ("is", n_("is")),
    ("isNot", n_("is not")),
    ("contains", n_("contains")),
    ("notContains", n_("does not contain")),
    ("startsWith", n_("starts with")),
    ("endsWith", n_("ends with")),
    ("gt", n_("more than")),
    ("lt", n_("less than")),
    ("before", n_("before")),
    ("after", n_("after")),
    ("inTheRange", n_("between")),
    ("inTheLast", n_("in the last")),
    ("notInTheLast", n_("not in the last")),
    ("inPlaylist", n_("in playlist")),
    ("notInPlaylist", n_("not in playlist")),
    ("isMissing", n_("is empty")),
    ("isPresent", n_("has a value")),
];

/// An operator's label as an English catalog key.
pub fn op_label(op: &str) -> &'static str {
    OP_LABELS.iter().find(|(k, _)| *k == op).map_or("", |(_, v)| v)
}

const TEXT_OPS: &[&str] = &["is", "isNot", "contains", "notContains", "startsWith", "endsWith"];
const MISSING_OPS: &[&str] = &["isPresent", "isMissing"];
const NUMBER_OPS: &[&str] = &["is", "isNot", "gt", "lt", "inTheRange"];

/// Which conditions Navidrome evaluates for a field.
/// Tags and roles take no ranges, and isMissing and isPresent need a nullable column, tag or role.
pub fn ops_for(f: &Field) -> Vec<&'static str> {
    let join = |a: &[&'static str], b: &[&'static str]| a.iter().chain(b).copied().collect::<Vec<_>>();
    match f.kind {
        Bool => vec!["is", "isNot"],
        Playlist => vec!["inPlaylist", "notInPlaylist"],
        Date => vec!["is", "isNot", "before", "after", "inTheRange", "inTheLast", "notInTheLast"],
        Tag | Genre => join(TEXT_OPS, MISSING_OPS),
        Text => if f.nullable { join(TEXT_OPS, MISSING_OPS) } else { TEXT_OPS.to_vec() },
        _ => if f.nullable { join(NUMBER_OPS, MISSING_OPS) } else { NUMBER_OPS.to_vec() },
    }
}

pub const SORT_RANDOM: &str = "random";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    None,
    Text,
    Number,
    Year,
    Days,
    Date,
    Bool,
    Genre,
    Playlist,
    RangeNumber,
    RangeYear,
    RangeDate,
}

impl Shape {
    pub fn is_range(self) -> bool {
        matches!(self, Shape::RangeNumber | Shape::RangeYear | Shape::RangeDate)
    }

    /// Keeps the characters this kind of value can contain.
    pub fn clean(self, v: &str) -> String {
        let digits = |max: usize| v.chars().filter(char::is_ascii_digit).take(max).collect::<String>();
        match self {
            Shape::Number | Shape::RangeNumber => v.chars().filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-').collect(),
            Shape::Year | Shape::RangeYear => digits(4),
            Shape::Days => digits(usize::MAX),
            Shape::Date | Shape::RangeDate => v.chars().filter(|c| c.is_ascii_digit() || *c == '-').take(10).collect(),
            _ => v.to_string(),
        }
    }

    /// The value field's placeholder, as an English catalog key.
    pub fn placeholder(self) -> &'static str {
        match self {
            Shape::Number | Shape::RangeNumber | Shape::Days => n_("Number"),
            Shape::Year | Shape::RangeYear => n_("Year"),
            Shape::Date | Shape::RangeDate => n_("YYYY-MM-DD"),
            _ => n_("Value"),
        }
    }
}

pub fn shape_of(f: &Field, op: &str) -> Shape {
    if op == "isMissing" || op == "isPresent" {
        return Shape::None;
    }
    let range = op == "inTheRange";
    match f.kind {
        Playlist => Shape::Playlist,
        Bool => Shape::Bool,
        Number => if range { Shape::RangeNumber } else { Shape::Number },
        Year => if range { Shape::RangeYear } else { Shape::Year },
        Date => match op {
            "inTheRange" => Shape::RangeDate,
            "inTheLast" | "notInTheLast" => Shape::Days,
            _ => Shape::Date,
        },
        Genre => if op == "is" || op == "isNot" { Shape::Genre } else { Shape::Text },
        _ => Shape::Text,
    }
}


/// A field id as Navidrome reads it, lowercased with older names mapped to current ones.
pub fn field_name(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    FIELD_ALIASES.iter().find(|(a, _)| *a == lower).map_or(lower.clone(), |(_, to)| to.to_string())
}

/// An operator in Navidrome's spelling, matched without regard to case.
pub fn op_name(raw: &str) -> Option<&'static str> {
    OP_LABELS.iter().find(|(k, _)| k.eq_ignore_ascii_case(raw)).map(|(k, _)| *k)
}

/// A sort direction in words for a field's type, as an English catalog key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Direction {
    pub desc: bool,
    pub label: &'static str,
}

const PLAY_COUNTS: &[&str] = &["playcount", "albumplaycount", "artistplaycount"];
const LENGTHS: &[&str] = &["duration", "albumduration"];

/// A field's two directions, the default first. Dates go newest first and text A to Z.
pub fn directions(field: &str) -> [Direction; 2] {
    let desc_first = |d: &'static str, a: &'static str| [Direction { desc: true, label: d }, Direction { desc: false, label: a }];
    if PLAY_COUNTS.contains(&field) {
        return desc_first(n_("Most played first"), n_("Least played first"));
    }
    if LENGTHS.contains(&field) {
        return desc_first(n_("Longest first"), n_("Shortest first"));
    }
    match field_by_id(field).map(|f| f.kind) {
        Some(Date | Year) => desc_first(n_("Newest first"), n_("Oldest first")),
        Some(Number) => desc_first(n_("Highest first"), n_("Lowest first")),
        Some(Bool) => desc_first(n_("Yes first"), n_("No first")),
        _ => [Direction { desc: false, label: n_("A–Z") }, Direction { desc: true, label: n_("Z–A") }],
    }
}

/// The words for one field and direction.
pub fn direction_label(field: &str, desc: bool) -> &'static str {
    let d = directions(field);
    if d[0].desc == desc { d[0].label } else { d[1].label }
}
