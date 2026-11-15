//! Home's Artist Radio row, one station per top artist, shown with similar library artists.
//! A seed needs [`MIN_NEIGHBOURS`] similar artists with their own photos, or it gets no station.

use std::collections::HashSet;

use crate::client::{Artist, Client};
use crate::deezer::Deezer;
use crate::library::Library;

/// Stations shown at most.
pub const STATIONS_MAX: usize = 10;
/// Top artists considered, so a few seeds without neighbors don't empty the row.
pub const CANDIDATES: usize = 20;
/// Neighbors with photos a station needs, for the two faces beside the seed's.
pub const MIN_NEIGHBOURS: usize = 2;

/// One station, the seed artist and its neighbors as library rows in merged order.
#[derive(Clone, Debug, PartialEq)]
pub struct Station {
    pub seed: (u32, Artist),
    pub neighbours: Vec<(u32, Artist)>,
}

/// Whether the server has a real photo for the artist.
/// Navidrome gives artists without one a placeholder, so the cover id tells them apart.
pub fn has_photo(artist: &Artist) -> bool {
    artist.cover_art.as_deref().is_some_and(|c| !c.is_empty())
}

/// The station for a seed, leaving out the seed itself, repeats and artists without a photo.
/// `None` with fewer than [`MIN_NEIGHBOURS`].
pub fn station(seed: (u32, Artist), similar: Vec<(u32, Artist)>) -> Option<Station> {
    let mut seen = HashSet::from([seed.0]);
    let neighbours: Vec<(u32, Artist)> = similar.into_iter().filter(|(row, a)| has_photo(a) && seen.insert(*row)).collect();
    (neighbours.len() >= MIN_NEIGHBOURS).then_some(Station { seed, neighbours })
}

/// Today's stations, best seed first. With `ask` false it reaches neither the server nor Deezer.
pub async fn artist_stations(client: &Client, db: &Library, deezer: &Deezer, ask: bool) -> Vec<Station> {
    let Ok(taste) = db.taste() else { return Vec::new() };
    let rowids: Vec<u32> = taste.artists.iter().take(CANDIDATES).map(|a| a.rowid).collect();
    let seeds = db.artists_by_rowid(&rowids).unwrap_or_default();
    let mut out = Vec::new();
    for (row, artist) in seeds {
        let similar = crate::similar_artists::similar_artists(client, db, deezer, &artist.id, ask).await;
        if let Some(found) = station((row, artist), similar) {
            out.push(found);
            if out.len() == STATIONS_MAX {
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artist(id: &str, photo: bool) -> Artist {
        Artist { id: id.into(), name: id.to_uppercase(), cover_art: photo.then(|| format!("ar-{id}")), ..Default::default() }
    }

    #[test]
    fn keeps_neighbours_with_photos_in_order_without_the_seed_or_repeats() {
        let seed = (1, artist("s", true));
        let similar = vec![(2, artist("a", true)), (1, artist("s", true)), (3, artist("b", false)), (4, artist("c", true)), (2, artist("a", true))];
        let found = station(seed, similar).unwrap();
        assert_eq!(found.neighbours.iter().map(|(r, _)| *r).collect::<Vec<_>>(), vec![2, 4]);
    }

    #[test]
    fn needs_two_neighbours_with_photos() {
        let similar = vec![(2, artist("a", true)), (3, artist("b", false))];
        assert!(station((1, artist("s", true)), similar).is_none());
        assert!(station((1, artist("s", true)), Vec::new()).is_none());
    }

    #[test]
    fn an_empty_cover_id_is_no_photo() {
        let mut a = artist("a", true);
        a.cover_art = Some(String::new());
        assert!(!has_photo(&a));
        assert!(has_photo(&artist("b", true)));
    }
}
