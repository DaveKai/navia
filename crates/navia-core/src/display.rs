//! How album names are shown.
//! Navidrome appends the version tag to album names by default, so Navia strips it and shows the version as a label.
//! Matching, sort keys and server calls keep the raw name.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::RwLock;

/// `name` without the " (<version>)" suffix Navidrome appends, matched at the end ignoring case.
pub fn album_name<'a>(name: &'a str, version: Option<&str>) -> Cow<'a, str> {
    let Some(version) = version.map(str::trim).filter(|v| !v.is_empty()) else { return Cow::Borrowed(name) };
    let bracketed = version.len() >= 2 && matches!((version.chars().next(), version.chars().last()), (Some('('), Some(')')) | (Some('['), Some(']')) | (Some('{'), Some('}')) | (Some('<'), Some('>')));
    let suffix = if bracketed { format!(" {version}") } else { format!(" ({version})") };
    let trimmed = name.trim_end();
    if trimmed.len() > suffix.len() && trimmed.is_char_boundary(trimmed.len() - suffix.len()) && trimmed[trimmed.len() - suffix.len()..].eq_ignore_ascii_case(&suffix) {
        let head = trimmed[..trimmed.len() - suffix.len()].trim_end();
        if !head.is_empty() {
            return Cow::Owned(head.to_string());
        }
    }
    Cow::Borrowed(name)
}

/// The version as a label beside the name, or `None` when there's none or it only says explicit or clean.
pub fn edition(version: Option<&str>) -> Option<String> {
    let version = version?.trim();
    let inner = version.strip_prefix(['(', '[']).and_then(|v| v.strip_suffix([')', ']'])).unwrap_or(version);
    let parts: Vec<&str> = inner.split(',').map(str::trim).filter(|p| !p.is_empty() && !is_rating_marker(p)).collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

fn is_rating_marker(part: &str) -> bool {
    matches!(part.to_ascii_lowercase().as_str(), "explicit" | "clean" | "explicit version" | "clean version")
}

/// Album versions by album id, since a song doesn't carry its album's version.
static VERSIONS: RwLock<Option<HashMap<String, String>>> = RwLock::new(None);

/// Records album versions. An empty or absent version removes one.
pub fn remember_versions<'a>(albums: impl IntoIterator<Item = (&'a str, Option<&'a str>)>) {
    let mut map = VERSIONS.write().expect("album versions");
    let map = map.get_or_insert_with(HashMap::new);
    for (id, version) in albums {
        match version.map(str::trim).filter(|v| !v.is_empty()) {
            Some(v) => {
                map.insert(id.to_string(), v.to_string());
            }
            None => {
                map.remove(id);
            }
        }
    }
}

/// A song's album name as shown, using its album's version when the library knows it.
pub fn song_album_name(album_id: Option<&str>, name: &str) -> String {
    let versions = VERSIONS.read().expect("album versions");
    let version = album_id.and_then(|id| versions.as_ref()?.get(id)).map(String::as_str);
    album_name(name, version).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_appended_version_goes() {
        assert_eq!(album_name("Hurry Up Tomorrow (explicit)", Some("explicit")), "Hurry Up Tomorrow");
        assert_eq!(album_name("Speak Your Mind (deluxe)", Some("deluxe")), "Speak Your Mind");
        assert_eq!(album_name("Speak Your Mind (Deluxe)", Some("deluxe")), "Speak Your Mind");
        assert_eq!(album_name("The Melodic Blue (Deluxe) (deluxe, explicit)", Some("deluxe, explicit")), "The Melodic Blue (Deluxe)");
        assert_eq!(album_name("R U Still Down? [Remember Me] (explicit)", Some("explicit")), "R U Still Down? [Remember Me]");
        assert_eq!(album_name("Album [Live]", Some("[Live]")), "Album");
    }

    #[test]
    fn names_without_the_version_stay() {
        assert_eq!(album_name("Album (Deluxe)", None), "Album (Deluxe)");
        assert_eq!(album_name("Album (Deluxe)", Some("")), "Album (Deluxe)");
        assert_eq!(album_name("Album (Deluxe)", Some("Remastered")), "Album (Deluxe)");
        assert_eq!(album_name("(explicit)", Some("explicit")), "(explicit)");
        assert_eq!(album_name("Albumexplicit)", Some("explicit")), "Albumexplicit)");
    }

    #[test]
    fn edition_leaves_out_explicit_and_clean() {
        assert_eq!(edition(Some("explicit")), None);
        assert_eq!(edition(Some("Clean")), None);
        assert_eq!(edition(Some("")), None);
        assert_eq!(edition(None), None);
        assert_eq!(edition(Some("deluxe, explicit")).as_deref(), Some("deluxe"));
        assert_eq!(edition(Some("Deluxe Edition")).as_deref(), Some("Deluxe Edition"));
        assert_eq!(edition(Some("[Live]")).as_deref(), Some("Live"));
    }

    #[test]
    fn songs_use_their_albums_version() {
        remember_versions([("al-test-1", Some("deluxe"))]);
        assert_eq!(song_album_name(Some("al-test-1"), "Speak Your Mind (deluxe)"), "Speak Your Mind");
        assert_eq!(song_album_name(Some("al-other"), "Speak Your Mind (deluxe)"), "Speak Your Mind (deluxe)");
        remember_versions([("al-test-1", None)]);
        assert_eq!(song_album_name(Some("al-test-1"), "Speak Your Mind (deluxe)"), "Speak Your Mind (deluxe)");
    }
}
