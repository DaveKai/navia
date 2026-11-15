//! Playlist Recommended, songs suggested under a playlist the user can edit.
//! Built only from `getSimilarSongs2` on a few of the playlist's songs, ranked by how well each fits.
//! Each song's score weighs seed agreement, genre, era and artist affinity, see [`WEIGHTS`].

use std::collections::{HashMap, HashSet};

use crate::autoplay::song_version;
use crate::client::{Client, Song};

/// Songs shown at once.
pub const SHOWN: usize = 10;
/// Seeds asked at once.
pub const SEEDS_PER_ASK: usize = 5;
/// Asks at most per batch, the first included.
pub const ROUNDS: usize = 3;
/// Songs per artist in one batch.
pub const ARTIST_MAX: usize = 2;
/// Similar songs asked for per seed.
pub const ANSWER_SIZE: u32 = 20;
/// The parts' weights: agreement, genre, era, artist. They add up to 1.
pub const WEIGHTS: Weights = Weights { agree: 0.30, genre: 0.25, era: 0.25, artist: 0.20 };
/// The score at which a song fits well.
pub const FIT_MIN: f32 = 0.5;
/// Seeds that make agreement full.
pub const AGREE_FULL: f32 = 3.0;
/// Years either side that count as the same era.
pub const ERA_WINDOW: i32 = 5;
/// Playlist artists listing an artist as similar that make artist affinity full.
pub const SIMILAR_FULL: f32 = 3.0;
/// A part's value when the song has no genre or year.
pub const UNKNOWN: f32 = 0.5;

#[derive(Clone, Copy, Debug)]
pub struct Weights {
    pub agree: f32,
    pub genre: f32,
    pub era: f32,
    pub artist: f32,
}

/// Similar library artist ids by playlist artist id.
pub type SimilarArtists = HashMap<String, Vec<String>>;

/// The artist a song counts under, its first credited artist's id, else its artist id, else its name.
pub fn artist_key(song: &Song) -> String {
    song.artists
        .first()
        .map(|a| a.id.clone())
        .filter(|id| !id.is_empty())
        .or_else(|| song.artist_id.clone().filter(|id| !id.is_empty()))
        .unwrap_or_else(|| crate::autoplay::main_artist(song).to_lowercase())
}

fn genre_of(song: &Song) -> &str {
    song.genre.as_deref().filter(|g| !g.is_empty()).or_else(|| song.genres.first().map(|g| g.name.as_str())).unwrap_or("")
}

fn year_of(song: &Song) -> Option<i32> {
    song.year.filter(|y| *y > 0)
}

fn genre_words(g: &str) -> Vec<String> {
    g.to_lowercase().split(|c: char| c.is_whitespace() || "/&,-".contains(c)).filter(|w| !w.is_empty()).map(str::to_string).collect()
}

/// 1 for the same genre, half for a shared word, else 0.
pub fn genre_match(a: &str, b: &str) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a.to_lowercase() == b.to_lowercase() {
        return 1.0;
    }
    let words = genre_words(a);
    if genre_words(b).iter().any(|w| words.contains(w)) { 0.5 } else { 0.0 }
}

/// The playlist's songs that seed suggestions, alternating most played and newest added, one per artist.
pub fn seeds(playlist: &[Song]) -> Vec<Song> {
    let newest: Vec<&Song> = playlist.iter().rev().collect();
    let mut played: Vec<&Song> = newest.iter().copied().filter(|s| s.play_count.unwrap_or(0) > 0).collect();
    played.sort_by_key(|s| std::cmp::Reverse(s.play_count.unwrap_or(0)));
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for k in 0..newest.len() {
        for s in [played.get(k).copied(), Some(newest[k])].into_iter().flatten() {
            if seen.insert(artist_key(s)) {
                out.push(s.clone());
            }
        }
    }
    out
}

/// What the playlist is like, from its songs alone.
struct Profile {
    genres: Vec<(String, f32)>,
    genre_top: f32,
    years: Vec<i32>,
    era_top: f32,
    artists: HashSet<String>,
    similar_to: HashMap<String, f32>,
}

impl Profile {
    fn of(playlist: &[Song], similar: &SimilarArtists) -> Self {
        let mut counts: Vec<(String, f32)> = Vec::new();
        for s in playlist {
            let g = genre_of(s);
            if g.is_empty() {
                continue;
            }
            match counts.iter_mut().find(|(pg, _)| pg == g) {
                Some((_, n)) => *n += 1.0,
                None => counts.push((g.to_string(), 1.0)),
            }
        }
        let share = |g: &str| counts.iter().map(|(pg, n)| n * genre_match(g, pg)).sum::<f32>();
        let genre_top = counts.iter().map(|(g, _)| share(g)).fold(1.0, f32::max);
        let years: Vec<i32> = playlist.iter().filter_map(year_of).collect();
        let near = |y: i32| years.iter().filter(|py| (**py - y).abs() <= ERA_WINDOW).count() as f32;
        let era_top = years.iter().map(|y| near(*y)).fold(1.0, f32::max);
        let artists: HashSet<String> = playlist.iter().map(artist_key).collect();
        let mut similar_to = HashMap::new();
        for a in &artists {
            for s in similar.get(a).into_iter().flatten() {
                *similar_to.entry(s.clone()).or_insert(0.0) += 1.0;
            }
        }
        Self { genres: counts, genre_top, years, era_top, artists, similar_to }
    }

    fn genre(&self, song: &Song) -> f32 {
        let g = genre_of(song);
        if g.is_empty() || self.genres.is_empty() {
            return UNKNOWN;
        }
        (self.genres.iter().map(|(pg, n)| n * genre_match(g, pg)).sum::<f32>() / self.genre_top).min(1.0)
    }

    fn era(&self, song: &Song) -> f32 {
        match year_of(song) {
            Some(y) if !self.years.is_empty() => self.years.iter().filter(|py| (**py - y).abs() <= ERA_WINDOW).count() as f32 / self.era_top,
            _ => UNKNOWN,
        }
    }

    fn artist(&self, song: &Song) -> f32 {
        let key = artist_key(song);
        if self.artists.contains(&key) { 1.0 } else { (self.similar_to.get(&key).copied().unwrap_or(0.0) / SIMILAR_FULL).min(1.0) }
    }
}

/// A candidate and its score.
#[derive(Clone, Debug)]
pub struct Scored {
    pub song: Song,
    pub score: f32,
}

/// Every candidate in the answers, best first, without the playlist's songs and their versions.
/// Ties keep the answers' order.
pub fn rank(playlist: &[Song], answers: &[Vec<Song>], similar: &SimilarArtists) -> Vec<Scored> {
    let have: HashSet<&str> = playlist.iter().map(|s| s.id.as_str()).collect();
    let versions: HashSet<String> = playlist.iter().map(song_version).collect();
    let mut votes: Vec<(Song, f32)> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    for answer in answers {
        let mut seen = HashSet::new();
        for s in answer {
            if !seen.insert(s.id.as_str()) || have.contains(s.id.as_str()) || versions.contains(&song_version(s)) {
                continue;
            }
            match at.get(&s.id) {
                Some(&i) => votes[i].1 += 1.0,
                None => {
                    at.insert(s.id.clone(), votes.len());
                    votes.push((s.clone(), 1.0));
                }
            }
        }
    }
    let fit = Profile::of(playlist, similar);
    let mut out: Vec<Scored> = votes
        .into_iter()
        .map(|(song, n)| {
            let score = WEIGHTS.agree * (n / AGREE_FULL).min(1.0) + WEIGHTS.genre * fit.genre(&song) + WEIGHTS.era * fit.era(&song) + WEIGHTS.artist * fit.artist(&song);
            Scored { song, score }
        })
        .collect();
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out
}

/// Up to [`SHOWN`] songs scoring at least `min`, skipping `skip`, repeats and an artist's songs past [`ARTIST_MAX`].
pub fn pick(ranked: &[Scored], skip: &HashSet<String>, min: f32) -> Vec<Song> {
    let mut out = Vec::new();
    let mut versions = HashSet::new();
    let mut per_artist: HashMap<String, usize> = HashMap::new();
    for c in ranked {
        if out.len() == SHOWN || c.score < min {
            break;
        }
        let artist = artist_key(&c.song);
        if skip.contains(&c.song.id) || per_artist.get(&artist).copied().unwrap_or(0) >= ARTIST_MAX || !versions.insert(song_version(&c.song)) {
            continue;
        }
        *per_artist.entry(artist).or_insert(0) += 1;
        out.push(c.song.clone());
    }
    out
}

/// Recommended for one visit to one playlist.
/// [`batch_for_playlist`] drives the whole flow of asking, answering and taking batches.
#[derive(Debug, Default)]
pub struct Recommender {
    seeds: Vec<Song>,
    asked: usize,
    asks_this_batch: usize,
    answers: Vec<Vec<Song>>,
    shown: HashSet<String>,
    on_screen: HashSet<String>,
}

impl Recommender {
    pub fn new(playlist: &[Song]) -> Self {
        Self { seeds: seeds(playlist), ..Default::default() }
    }

    /// The seed song ids to ask for next, or `None` when this batch has asked enough.
    pub fn wanted(&self, playlist: &[Song], similar: &SimilarArtists) -> Option<Vec<String>> {
        if self.asks_this_batch >= ROUNDS || self.asked >= self.seeds.len() {
            return None;
        }
        if self.asked > 0 && pick(&rank(playlist, &self.answers, similar), &self.shown, FIT_MIN).len() >= SHOWN {
            return None;
        }
        Some(self.seeds[self.asked..(self.asked + SEEDS_PER_ASK).min(self.seeds.len())].iter().map(|s| s.id.clone()).collect())
    }

    /// The server's answers for the seeds [`Recommender::wanted`] gave, one list per seed.
    pub fn answered(&mut self, answers: Vec<Vec<Song>>) {
        self.asked = (self.asked + answers.len()).min(self.seeds.len());
        self.asks_this_batch += 1;
        self.answers.extend(answers);
    }

    /// The next batch, the best not shown this visit. Once all were shown, the best except those on screen.
    pub fn take_batch(&mut self, playlist: &[Song], similar: &SimilarArtists) -> Vec<Song> {
        let ranked = rank(playlist, &self.answers, similar);
        let mut songs = pick(&ranked, &self.shown, 0.0);
        if songs.is_empty() {
            songs = pick(&ranked, &self.on_screen, 0.0);
        }
        self.asks_this_batch = 0;
        self.on_screen = songs.iter().map(|s| s.id.clone()).collect();
        self.shown.extend(self.on_screen.iter().cloned());
        songs
    }
}

/// Asks the server for these seeds' similar songs, all at once. Read-only.
pub async fn fetch(client: &Client, seeds: &[String]) -> Vec<Vec<Song>> {
    crate::autoplay::fetch_count(client, seeds, ANSWER_SIZE).await
}

/// The next batch for a stored playlist, keeping the visit's recommender in `state`.
/// The lock isn't held while the server answers.
pub async fn batch_for_playlist(client: &Client, db: &crate::library::Library, playlist_id: &str, state: &std::sync::Mutex<Option<Recommender>>) -> Vec<Song> {
    let playlist = db.playlist(playlist_id).ok().flatten().map(|p| p.entry).unwrap_or_default();
    let artists: Vec<String> = playlist.iter().map(artist_key).collect::<HashSet<_>>().into_iter().collect();
    let similar = db.similar_stored(&artists).unwrap_or_default();
    loop {
        let ids = {
            let mut guard = state.lock().unwrap();
            guard.get_or_insert_with(|| Recommender::new(&playlist)).wanted(&playlist, &similar)
        };
        let Some(ids) = ids else { break };
        let answers = fetch(client, &ids).await;
        if let Some(rec) = state.lock().unwrap().as_mut() {
            rec.answered(answers);
        }
    }
    state.lock().unwrap().as_mut().map(|rec| rec.take_batch(&playlist, &similar)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ArtistRef;

    fn song(id: &str, artist: &str, genre: &str, year: i32) -> Song {
        Song {
            id: id.into(),
            title: format!("Song {id}"),
            artists: vec![ArtistRef { id: format!("ar-{artist}"), name: artist.into() }],
            genre: Some(genre.into()).filter(|g: &String| !g.is_empty()),
            year: Some(year).filter(|y| *y > 0),
            ..Default::default()
        }
    }

    fn eighties() -> Vec<Song> {
        (0..10).map(|i| song(&format!("p{i}"), &format!("P{}", i % 5), "Pop", 1982 + i % 6)).collect()
    }

    #[test]
    fn seeds_alternate_most_played_and_newest_one_per_artist() {
        let mut list = vec![song("a", "A", "Pop", 1980), song("b", "B", "Pop", 1980), song("c", "A", "Pop", 1980), song("d", "C", "Pop", 1980)];
        list[1].play_count = Some(9);
        let ids: Vec<String> = seeds(&list).into_iter().map(|s| s.id).collect();
        // Most played b, then newest d, then c as the newest of artist A, never a, since A is taken.
        assert_eq!(ids, ["b", "d", "c"]);
    }

    #[test]
    fn genre_match_counts_a_shared_word_half() {
        assert_eq!(genre_match("Pop", "pop"), 1.0);
        assert_eq!(genre_match("Dance-Pop", "Pop"), 0.5);
        assert_eq!(genre_match("Rock", "Jazz"), 0.0);
        assert_eq!(genre_match("", "Jazz"), 0.0);
    }

    #[test]
    fn era_and_genre_rank_a_fitting_song_over_a_far_one() {
        let list = eighties();
        let near = song("near", "X", "Pop", 1984);
        let far = song("far", "Y", "Hip Hop", 2022);
        let ranked = rank(&list, &[vec![far.clone(), near.clone()]], &SimilarArtists::new());
        assert_eq!(ranked[0].song.id, "near");
        assert!(ranked[0].score >= FIT_MIN, "{}", ranked[0].score);
        assert!(ranked[1].score < FIT_MIN, "{}", ranked[1].score);
    }

    #[test]
    fn agreement_and_artist_affinity_lift_a_song() {
        let list = eighties();
        let once = song("once", "X", "Pop", 1984);
        let thrice = song("thrice", "Y", "Pop", 1984);
        let answers = vec![vec![once.clone(), thrice.clone()], vec![thrice.clone()], vec![thrice.clone()]];
        let ranked = rank(&list, &answers, &SimilarArtists::new());
        assert_eq!(ranked[0].song.id, "thrice");
        // X listed as similar by three playlist artists gives full artist affinity.
        let similar: SimilarArtists = (0..3).map(|i| (format!("ar-P{i}"), vec!["ar-X".to_string()])).collect();
        let lifted = rank(&list, &[vec![once.clone()]], &similar);
        let plain = rank(&list, &[vec![once]], &SimilarArtists::new());
        assert!((lifted[0].score - plain[0].score - WEIGHTS.artist).abs() < 1e-5);
    }

    #[test]
    fn missing_genre_and_year_are_neutral() {
        let list = eighties();
        let bare = song("bare", "X", "", 0);
        let s = rank(&list, &[vec![bare]], &SimilarArtists::new())[0].score;
        let expected = WEIGHTS.agree / AGREE_FULL + (WEIGHTS.genre + WEIGHTS.era) * UNKNOWN;
        assert!((s - expected).abs() < 1e-5, "{s}");
    }

    #[test]
    fn drops_playlist_songs_their_versions_and_caps_artists() {
        let list = eighties();
        let mut live = list[0].clone();
        live.id = "live".into();
        live.title = format!("{} (Live)", live.title);
        let answer: Vec<Song> = vec![list[1].clone(), live].into_iter().chain((0..4).map(|i| song(&format!("z{i}"), "Z", "Pop", 1984))).collect();
        let picked = pick(&rank(&list, &[answer], &SimilarArtists::new()), &HashSet::new(), 0.0);
        let ids: Vec<&str> = picked.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["z0", "z1"]);
    }

    #[test]
    fn asks_more_seeds_only_while_too_few_fit_and_fills_with_the_rest() {
        let list: Vec<Song> = (0..30).map(|i| song(&format!("p{i}"), &format!("P{i}"), "Pop", 1984)).collect();
        let mut rec = Recommender::new(&list);
        let similar = SimilarArtists::new();
        let first = rec.wanted(&list, &similar).unwrap();
        assert_eq!(first.len(), SEEDS_PER_ASK);
        // Poor answers suggest far songs, so it keeps asking up to ROUNDS times.
        let mut asks = 0;
        while let Some(ids) = rec.wanted(&list, &similar) {
            asks += 1;
            let answers = ids.iter().map(|id| (0..3).map(|k| song(&format!("{id}-far{k}"), &format!("F{id}{k}"), "Metal", 2023)).collect()).collect();
            rec.answered(answers);
        }
        assert_eq!(asks, ROUNDS);
        // The best of the rest fill the batch.
        assert_eq!(rec.take_batch(&list, &similar).len(), SHOWN);
    }

    #[test]
    fn refresh_never_repeats_until_all_were_shown() {
        let list = eighties();
        let mut rec = Recommender::new(&list);
        let similar = SimilarArtists::new();
        let pool: Vec<Song> = (0..25).map(|i| song(&format!("c{i}"), &format!("C{i}"), "Pop", 1984)).collect();
        while rec.wanted(&list, &similar).is_some() {
            let n = rec.wanted(&list, &similar).unwrap().len();
            rec.answered(vec![pool.clone(); n]);
        }
        let a = rec.take_batch(&list, &similar);
        let b = rec.take_batch(&list, &similar);
        let c = rec.take_batch(&list, &similar);
        assert_eq!((a.len(), b.len(), c.len()), (10, 10, 5));
        let all: HashSet<&str> = a.iter().chain(&b).chain(&c).map(|s| s.id.as_str()).collect();
        assert_eq!(all.len(), 25);
        // With everything shown, it starts over without the five on screen.
        let d = rec.take_batch(&list, &similar);
        assert!(d.iter().all(|s| !c.iter().any(|x| x.id == s.id)));
        assert_eq!(d.len(), 10);
    }

    #[test]
    fn empty_answers_give_nothing() {
        let list = eighties();
        let mut rec = Recommender::new(&list);
        let similar = SimilarArtists::new();
        while let Some(ids) = rec.wanted(&list, &similar) {
            rec.answered(vec![Vec::new(); ids.len()]);
        }
        assert!(rec.take_batch(&list, &similar).is_empty());
    }
}
