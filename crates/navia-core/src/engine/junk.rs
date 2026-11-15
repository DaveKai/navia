//! What Autoplay never picks by itself: very short tracks and versions without the lead vocal,
//! such as instrumentals, karaoke or stems. Songs in the list being played are never touched.
//!
//! Versions come from the title's and album's decorations, never the bare title, so a song
//! called "Karaoke" stays. A kind the listener was just playing is allowed.

use crate::client::Song;
use crate::lanyard::matching::key;

/// Below this, a track is far more often an intro, interlude, skit or jingle than a song.
pub const SHORT_SECS: i64 = 60;

/// A version without the song's lead vocal, or with only it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Variant {
    Instrumental,
    Karaoke,
    /// A cappella, isolated vocals, vocals only, stems.
    VocalsOnly,
}

/// Word sequences (as [`key`] gives them) that mark a variant. Matched as whole words.
const WORDS: &[(&str, Variant)] = &[
    ("instrumental", Variant::Instrumental),
    ("instrumentals", Variant::Instrumental),
    ("instrumentale", Variant::Instrumental),
    ("instrumentales", Variant::Instrumental),
    ("instrumentaal", Variant::Instrumental),
    ("instrumentalversion", Variant::Instrumental),
    ("off vocal", Variant::Instrumental),
    ("offvocal", Variant::Instrumental),
    ("no vocal", Variant::Instrumental),
    ("no vocals", Variant::Instrumental),
    ("without vocals", Variant::Instrumental),
    ("backing track", Variant::Instrumental),
    ("minus one", Variant::Instrumental),
    ("karaoke", Variant::Karaoke),
    ("karaoké", Variant::Karaoke),
    ("sing along version", Variant::Karaoke),
    ("a cappella", Variant::VocalsOnly),
    ("a capella", Variant::VocalsOnly),
    ("acappella", Variant::VocalsOnly),
    ("acapella", Variant::VocalsOnly),
    ("isolated vocal", Variant::VocalsOnly),
    ("isolated vocals", Variant::VocalsOnly),
    ("vocals only", Variant::VocalsOnly),
    ("vocal only", Variant::VocalsOnly),
    ("stems", Variant::VocalsOnly),
];

/// Short markers that are also names or words, like "Mr. X". Only match a whole decoration,
/// optionally followed by "ver" or "version".
const WHOLE: &[(&str, Variant)] = &[("inst", Variant::Instrumental), ("mr", Variant::Instrumental), ("stem", Variant::VocalsOnly)];

/// Words allowed after "karaoke" in one decoration, such as "Karaoke Version". Anything else
/// means it's part of a name. Other markers aren't held to this.
const AFTER: &[&str] = &[
    "version", "versions", "ver", "mix", "edit", "track", "take", "demo", "radio", "sped", "up", "slowed", "reverb", "remaster", "remastered", "original", "single", "album", "backing", "with", "w", "alternative", "alternate", "vocal", "vocals", "only", "and", "band", "full", "short", "extended",
];

/// Markers in scripts without spaces between words, matched anywhere in a decoration.
const SCRIPTS: &[(&str, Variant)] = &[
    ("カラオケ", Variant::Karaoke),
    ("オフボーカル", Variant::Instrumental),
    ("インスト", Variant::Instrumental),
    ("伴奏", Variant::Instrumental),
    ("纯音乐", Variant::Instrumental),
    ("純音樂", Variant::Instrumental),
    ("인스트", Variant::Instrumental),
    ("반주", Variant::Instrumental),
];

/// The decorations of a title or album name: what's inside brackets and what follows a spaced dash.
fn decorations(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for c in text.chars() {
        match c {
            '(' | '[' | '{' | '（' | '【' | '［' | '「' => {
                if depth > 0 && !current.trim().is_empty() {
                    out.push(std::mem::take(&mut current));
                }
                depth += 1;
                current.clear();
            }
            ')' | ']' | '}' | '）' | '】' | '］' | '」' if depth > 0 => {
                depth -= 1;
                out.push(std::mem::take(&mut current));
            }
            _ if depth > 0 => current.push(c),
            _ => {}
        }
    }
    if depth > 0 {
        out.push(current);
    }
    // Japanese releases set a version between dashes: "Title -Instrumental-".
    if let Some(inner) = text.trim_end().strip_suffix('-').and_then(|t| t.rsplit_once(" -")).map(|(_, inner)| inner) {
        out.push(inner.to_string());
    }
    for dash in [" - ", " – ", " — ", " ~ "] {
        if let Some((_, after)) = text.split_once(dash) {
            out.push(after.to_string());
        }
    }
    out.retain(|d| !d.trim().is_empty());
    out
}

/// The variant a decoration names. `strict` is for albums, where "(Main + Instrumental)" also
/// holds the song itself, so only the marker and [`AFTER`] words may appear.
fn variant_in(decoration: &str, strict: bool) -> Option<Variant> {
    if let Some((_, v)) = SCRIPTS.iter().find(|(m, _)| decoration.contains(m)) {
        return Some(*v);
    }
    let folded = key(decoration);
    let whole = folded.strip_suffix(" version").or_else(|| folded.strip_suffix(" ver")).unwrap_or(&folded);
    if let Some((_, v)) = WHOLE.iter().find(|(m, _)| *m == whole) {
        return Some(*v);
    }
    let words: Vec<&str> = folded.split(' ').collect();
    WORDS
        .iter()
        .find(|(m, _)| {
            let marker = key(m);
            let marker: Vec<&str> = marker.split(' ').collect();
            let named = |rest: &[&str]| rest.iter().any(|a| !AFTER.contains(a) && !a.chars().all(|c| c.is_ascii_digit()));
            words.windows(marker.len()).enumerate().any(|(i, w)| {
                let after = &words[i + marker.len()..];
                w == marker.as_slice() && !(marker[0] == "karaoke" && named(after)) && !(strict && (named(&words[..i]) || named(after)))
            })
        })
        .map(|(_, v)| *v)
}

/// The variant a song is, from its title's decorations, else its album's.
pub fn variant(song: &Song) -> Option<Variant> {
    decorations(&song.title).iter().find_map(|d| variant_in(d, false)).or_else(|| song.album.as_deref().and_then(|a| decorations(a).iter().find_map(|d| variant_in(d, true))))
}

/// Shorter than [`SHORT_SECS`]. A song with no length isn't.
pub fn is_short(song: &Song) -> bool {
    song.duration.is_some_and(|d| d > 0 && d < SHORT_SECS)
}

/// Which songs Autoplay leaves out, given what was playing before it.
#[derive(Clone, Debug, Default)]
pub struct Filter {
    /// Variants the listener was playing just before, so Autoplay may continue with them.
    allowed: Vec<Variant>,
}

impl Filter {
    /// `before` is the last songs of the queue.
    pub fn after(before: &[Song]) -> Self {
        let mut allowed: Vec<Variant> = before.iter().filter_map(variant).collect();
        allowed.dedup();
        Self { allowed }
    }

    /// Whether Autoplay should leave this song out.
    pub fn leaves_out(&self, song: &Song) -> bool {
        is_short(song) || variant(song).is_some_and(|v| !self.allowed.contains(&v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titled(title: &str) -> Song {
        Song { title: title.into(), duration: Some(200), ..Default::default() }
    }

    #[test]
    fn short_tracks_are_left_out() {
        let f = Filter::default();
        assert!(f.leaves_out(&Song { title: "Intro".into(), duration: Some(13), ..Default::default() }));
        assert!(f.leaves_out(&Song { title: "Song".into(), duration: Some(59), ..Default::default() }));
        assert!(!f.leaves_out(&Song { title: "Song".into(), duration: Some(60), ..Default::default() }));
        assert!(!f.leaves_out(&Song { title: "Song".into(), duration: None, ..Default::default() }), "no length known: kept");
    }

    #[test]
    fn variants_without_the_lead_vocal_are_left_out() {
        let f = Filter::default();
        for title in [
            "Skyfall (instrumental)",
            "Square Dance - Instrumental",
            "Seven Seas of Rhye (instrumental mix 2011)",
            "BLUE (ISOLATED VOCALS)",
            "Song [Karaoke Version]",
            "Song (A Cappella)",
            "Song (Acapella)",
            "Song - Vocals Only",
            "Song (Stems)",
            "Canción (Instrumentale)",
            "Lied (Instrumental-Version)",
            "Song (Inst.)",
            "노래 (Inst.)",
            "Song (MR)",
            "歌 (Off Vocal)",
            "歌 (オリジナル・カラオケ)",
            "歌【オフボーカル】",
            "歌（伴奏）",
            "歌 [纯音乐]",
            "Song (Backing Track)",
            "Revolution (Take 14 / Instrumental Backing Track)",
            "Oscillate Wildly (Instrumental; 2008 Remaster)",
            "Songbird (instrumental, take 10)",
            "Nightcrawler (Instrumental Sped Up)",
            "Just To Keep You Satisfied - A Cappella W/ Alternative Vocal",
            "São Paulo (feat. Anitta) - Single Version / Acapella",
            "Bohemian Rhapsody (operatic section a cappella mix 2011)",
            "Babble - Alternative Version / Studio Instrumental Rough",
            "Fear Of Ghosts - Band Instrumental Rehearsal",
            "Memories of You -Reload Instrumental-",
        ] {
            assert!(f.leaves_out(&titled(title)), "{title}");
        }
        let on_album = Song { title: "Lose Yourself".into(), album: Some("Curtain Call (Instrumentals)".into()), duration: Some(300), ..Default::default() };
        assert!(f.leaves_out(&on_album));
        let both = Song { title: "Die With A Smile".into(), album: Some("Die With A Smile (Main + Instrumental)".into()), duration: Some(251), ..Default::default() };
        assert!(!f.leaves_out(&both), "an album holding the song and its instrumental");
    }

    #[test]
    fn songs_that_only_sound_alike_stay() {
        let f = Filter::default();
        for title in [
            "Karaoke",
            "Instrumental",
            "Running Away (vocal remix)",
            "I Was Born to Love You (vocal & piano version)",
            "Ride the Wild Wind (early version with guide vocal)",
            "I’m in Love With My Car (guitar & vocal mix 2011)",
            "Instrumentalist",
            "Mr. Brightside",
            "Stem the Tide",
            "Song (feat. Mr. Hudson)",
            "Song (with Stem)",
            "卡拉OK(Karaoke Dreams)",
            "Dreams (Live)",
            "Mr. Blue Sky - Remastered",
        ] {
            assert!(!f.leaves_out(&titled(title)), "{title}");
        }
    }

    #[test]
    fn a_kind_the_listener_was_playing_may_continue() {
        let before = [titled("Skyfall (instrumental)"), titled("Song")];
        let f = Filter::after(&before);
        assert!(!f.leaves_out(&titled("Hymn for the Weekend (instrumental)")));
        assert!(f.leaves_out(&titled("Song (Karaoke)")), "only the kind that was playing");
        assert!(f.leaves_out(&Song { title: "Intro".into(), duration: Some(20), ..Default::default() }), "short stays out");
    }
}
