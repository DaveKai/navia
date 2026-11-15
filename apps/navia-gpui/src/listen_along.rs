//! Same-server listen-along, opt-in on both sides. The rules live in `navia_core::listen_along`.
//! This keeps "Listening Now" fresh, plays the followed song in place of the queue, which waits
//! untouched, and restores the queue when following ends.

use std::time::{Duration, Instant};

use gpui::Context;
use navia_core::audio::player::{PlayerStatus, QueueItem};
use navia_core::lanyard::{self as ly, FriendNow};
use navia_core::listen_along::{self as la, FollowStep, Followed, NowPlaying};

use crate::i18n::trf;
use crate::shell::Shell;

/// Listening Now and following, for the signed-in account.
#[derive(Default)]
pub struct ListenState {
    /// The people listed in "Listening Now".
    pub listing: Vec<NowPlaying>,
    /// When `listing` was last read from the server. It's dropped once older than `la::LISTING_KEEP`.
    pub listing_at: Option<Instant>,
    pub following: Option<Following>,
    /// "<user> stopped listening", shown for a few seconds after following ends on its own.
    pub ended: Option<String>,
    ended_generation: u64,
    /// When the next check is due.
    next_check: Option<Instant>,
    checking: bool,
    /// Where the listener's own song resumes after following, as its row, queue place and position.
    resume: Option<(u32, usize, u64)>,
    /// Spotify friends as Lanyard last reported them, with songs matched in the library.
    pub friends: Vec<FriendNow>,
    /// The watch generation `friends` was read at.
    friends_generation: Option<u64>,
    friends_checking: bool,
    /// The friend's song last skipped as missing from the library, so the notice shows once.
    skipped: Option<String>,
    /// The Lanyard address given to the watch, so an edited one applies once typing is done.
    applied_address: Option<String>,
    /// The Spotify friend being added in Settings.
    pub draft_id: String,
    pub draft_name: String,
    /// People listening along with this Navia at the last check, kept only while listeners may
    /// control playback, and when this user last changed playback.
    pub listeners: Vec<la::Listener>,
    pub host_change_ms: u64,
    /// The marks the reports carry, as control and following.
    marks: Option<(bool, bool)>,
}

/// The person followed and their song as it plays here.
pub struct Following {
    pub followed: Followed,
    pub song: NowPlaying,
    /// The song's row in this library, if any. It plays either way.
    pub rowid: Option<u32>,
    /// Where the listener's own song was, so it resumes from there.
    pub back_ms: u64,
    /// Following a Spotify friend whose song isn't in the library. Nothing plays until they play
    /// one that is.
    pub waiting: bool,
    /// The listener paused it, so it stays paused and following until they play again.
    pub held: bool,
    /// When the listener last changed playback while following, and the followed player's state at
    /// the last check.
    pub own_change_ms: Option<u64>,
    pub seen: Option<(bool, u64)>,
}

impl Shell {
    /// Whether "Let listeners control playback" applies, needing sharing and the setting on.
    pub fn listeners_control(&self) -> bool {
        self.listen_active() && self.prefs.listeners_control
    }

    /// Whether the followed person lets listeners control playback.
    pub fn along_controls(&self) -> bool {
        self.listen.following.as_ref().is_some_and(|f| ly::friend_of(&f.followed.player).is_none() && f.song.allows_control())
    }

    /// Updates the reports' marks for the setting and whether this Navia follows someone.
    pub fn apply_listen_marks(&mut self) {
        let control = self.listeners_control();
        let following = self.listen.following.as_ref().is_some_and(|f| ly::friend_of(&f.followed.player).is_none());
        if self.listen.marks == Some((control, following)) {
            return;
        }
        self.listen.marks = Some((control, following));
        if let Some(core) = self.core.as_ref() {
            let lib = core.lib.clone();
            std::thread::spawn(move || lib.set_listen_marks(control, following));
        }
        if !control {
            self.listen.listeners.clear();
        }
    }

    /// Handles play or pause while following, keeping the follow. A pause holds and play catches up
    /// at once. If the host lets listeners control playback, their Navia mirrors it.
    pub fn along_toggle(&mut self, play: Option<bool>, cx: &mut Context<Self>) -> bool {
        let controls = self.along_controls();
        let Some(f) = self.listen.following.as_mut() else { return false };
        let play = play.unwrap_or(!self.player.playing);
        f.held = !play;
        // Only a host that lets listeners control playback mirrors it. Otherwise play catches up at once.
        f.own_change_ms = controls.then(ly::now_ms);
        if let Some(core) = self.core.as_ref() {
            if play { core.player.play() } else { core.player.pause() }
        }
        self.player.playing = play;
        if play {
            self.check_listening(cx);
        }
        cx.notify();
        true
    }

    /// Seeks while following a host that lets listeners control playback, and their Navia mirrors
    /// it. Returns false otherwise, and seeking ends following.
    pub fn along_seek(&mut self, ms: u64, cx: &mut Context<Self>) -> bool {
        if !self.along_controls() {
            return false;
        }
        if let Some(f) = self.listen.following.as_mut() {
            f.own_change_ms = Some(ly::now_ms());
        }
        if let Some(core) = self.core.as_ref() {
            core.player.seek_ms(ms);
        }
        self.player.position = ms as f32 / 1000.0;
        cx.notify();
        true
    }

    /// Plays a song while following a host that lets listeners control playback, and their Navia
    /// plays it too. Returns false otherwise, and playing something else ends following.
    pub fn along_play_song(&mut self, rowid: u32, cx: &mut Context<Self>) -> bool {
        if !self.along_controls() {
            return false;
        }
        let Some(core) = self.core.as_ref() else { return false };
        let Some((_, song)) = core.lib.db.songs_by_rowid(&[rowid]).ok().and_then(|r| r.into_iter().next()) else { return false };
        let Some(f) = self.listen.following.as_mut() else { return false };
        f.own_change_ms = Some(ly::now_ms());
        f.held = false;
        let entry = NowPlaying { song, state: Some(la::SessionState::Playing), position_ms: Some(0), ..f.song.clone() };
        f.song = entry.clone();
        self.load_followed(&entry, cx);
        true
    }

    /// Next or Previous while following a host that allows control. Next plays the listener's own
    /// waiting song on both sides, Previous restarts the song.
    pub fn along_skip(&mut self, forward: bool, cx: &mut Context<Self>) -> bool {
        if !self.along_controls() {
            return false;
        }
        if !forward {
            return self.along_seek(0, cx);
        }
        let Some(&rowid) = self.player.queue.get(self.player.queue_pos) else { return false };
        if !self.along_play_song(rowid, cx) {
            return false;
        }
        // The queue waits under the followed song, and its place moves past the song played.
        if let Some(core) = self.core.as_mut() {
            core.pq.step_waiting();
        }
        self.mirror_queue();
        true
    }

    /// The host side when not following anyone. Tracks who listens along and, if allowed, mirrors
    /// their changes here.
    fn host_check(&mut self, entries: &[NowPlaying], cx: &mut Context<Self>) {
        if !self.listeners_control() || self.listen.following.is_some() {
            self.listen.listeners.clear();
            return;
        }
        let Some(core) = self.core.as_ref() else { return };
        let Some(song_id) = self.player.current.as_ref().map(|s| s.id.to_string()) else {
            self.listen.listeners.clear();
            return;
        };
        let viewer = core.lib.username().to_string();
        let host = la::Host { song_id, position_ms: (self.player.position.max(0.0) * 1000.0) as u64, playing: self.player.playing, own_change_ms: self.listen.host_change_ms };
        let now = la::listeners(entries, &viewer, &host, &self.listen.listeners, ly::now_ms());
        let action = la::host_mirror(&host, &self.listen.listeners, &now, true);
        self.listen.listeners = now;
        let Some(action) = action else { return };
        let player = core.player.clone();
        match action {
            la::HostAction::Pause => player.pause(),
            la::HostAction::Play => player.play(),
            la::HostAction::Seek(ms) => {
                player.seek_ms(ms);
                self.player.position = ms as f32 / 1000.0;
            }
            la::HostAction::Song { song_id, position_ms, playing } => {
                let Some(rowid) = core.lib.rowid_for_song(&song_id) else { return };
                // Plays now as a queued song, and the host's queue continues after it.
                self.queue_songs_at(&[rowid], 0, true, cx);
                let Some(core) = self.core.as_ref() else { return };
                core.player.next();
                if position_ms > la::RESYNC_DRIFT.as_millis() as u64 {
                    core.player.seek_ms(position_ms);
                }
                if playing { core.player.play() } else { core.player.pause() }
            }
        }
        cx.notify();
    }

    /// Records that this user changed playback, for last-change-wins with listeners.
    pub fn note_host_change(&mut self) {
        self.listen.host_change_ms = ly::now_ms();
    }
    /// Others are listed only while this user shares too, with sharing on, Private mode off and a
    /// connection.
    pub fn listen_active(&self) -> bool {
        self.core.is_some() && self.prefs.share_listening && !self.private_mode && !self.is_offline()
    }

    /// Spotify friends show whenever there's a library and the server is reachable. Private mode
    /// doesn't hide them, since following them reveals nothing.
    pub fn friends_active(&self) -> bool {
        self.core.is_some() && !self.is_offline()
    }

    /// The Spotify friends whose activity shows a song.
    pub fn listening_friends(&self) -> Vec<FriendNow> {
        if !self.friends_active() {
            return Vec::new();
        }
        self.listen.friends.iter().filter(|f| f.listening()).cloned().collect()
    }

    /// Adds the typed Discord id and optional name as a Spotify friend. Returns whether it was added.
    pub fn add_friend(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(id) = ly::parse_friend_id(&self.listen.draft_id) else { return false };
        if self.prefs.spotify_friends.iter().any(|f| f.id == id) {
            return false;
        }
        let name = Some(self.listen.draft_name.trim().to_string()).filter(|n| !n.is_empty());
        self.prefs.spotify_friends.push(ly::Friend { id, name });
        self.prefs.save();
        self.listen.draft_id.clear();
        self.listen.draft_name.clear();
        self.controls.text = None;
        self.apply_friends(cx);
        true
    }

    /// Removes a Spotify friend.
    pub fn remove_friend(&mut self, id: &str, cx: &mut Context<Self>) {
        self.prefs.spotify_friends.retain(|f| f.id != id);
        self.prefs.save();
        self.listen.friends.retain(|f| f.friend.id != id);
        self.apply_friends(cx);
    }

    /// The followed person, if it's a Spotify friend.
    pub fn following_friend(&self) -> Option<&str> {
        self.listen.following.as_ref().and_then(|f| ly::friend_of(&f.followed.player))
    }

    /// Whether the friend with this Discord id is followed now.
    pub fn following_friend_id(&self, id: &str) -> bool {
        self.following_friend() == Some(id)
    }

    /// Applies changed Spotify friend settings or a newly opened account to the watch. A friend
    /// removed while followed stops being followed.
    pub fn apply_friends(&mut self, cx: &mut Context<Self>) {
        if let Some(core) = self.core.as_ref() {
            core.lib.configure_friends(&self.prefs.lanyard_address, self.prefs.spotify_friends.clone());
            self.listen.applied_address = Some(self.prefs.lanyard_address.clone());
        }
        if let Some(id) = self.following_friend().map(str::to_string)
            && !self.prefs.spotify_friends.iter().any(|f| f.id == id)
        {
            self.stop_listening(false, cx);
        }
        self.listen.friends_generation = None;
        cx.notify();
    }

    /// Applies a change to sharing or Private mode. The reports' mark follows the setting, and
    /// turning sharing off ends following.
    pub fn apply_sharing(&mut self, cx: &mut Context<Self>) {
        if let Some(core) = self.core.as_ref() {
            let (lib, on) = (core.lib.clone(), self.prefs.share_listening);
            cx.background_executor().spawn(async move { lib.set_sharing(on) }).detach();
        }
        if !self.listen_active() {
            if self.listen.following.is_some() && self.following_friend().is_none() {
                self.stop_listening(false, cx);
            }
            self.listen.listing.clear();
        } else {
            self.listen.next_check = None;
        }
        cx.notify();
    }

    /// Starts the checks for an account. They end with it.
    pub fn start_listen_checks(&mut self, generation: u64, cx: &mut Context<Self>) {
        self.listen = ListenState::default();
        self.apply_friends(cx);
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let alive = this.update(cx, |shell, cx| {
                if shell.core_generation != generation {
                    return false;
                }
                shell.listen_tick(cx);
                true
            });
            if !matches!(alive, Ok(true)) {
                return;
            }
        })
        .detach();
    }

    fn listen_tick(&mut self, cx: &mut Context<Self>) {
        self.friends_tick(cx);
        self.apply_listen_marks();
        if !self.listen_active() {
            if !self.listen.listing.is_empty() {
                self.listen.listing.clear();
                cx.notify();
            }
            return;
        }
        // Checks that keep failing leave nothing behind that looks current.
        if !self.listen.listing.is_empty() && self.listen.listing_at.is_none_or(|at| at.elapsed() > la::LISTING_KEEP) {
            self.listen.listing.clear();
            cx.notify();
        }
        if self.listen.next_check.is_none_or(|at| Instant::now() >= at) {
            self.check_listening(cx);
        }
    }

    /// Rereads Spotify friends whenever Lanyard reports something new, and every second while
    /// following one. The watch holds presences and matching reads the local library, so there's no
    /// network call here.
    fn friends_tick(&mut self, cx: &mut Context<Self>) {
        let typing = self.focused_target() == Some(crate::ui_controls::TextTarget::LanyardAddress);
        if !typing && self.listen.applied_address.as_deref() != Some(self.prefs.lanyard_address.as_str()) {
            self.apply_friends(cx);
        }
        let Some(core) = self.core.as_ref() else { return };
        let following = self.following_friend().is_some();
        core.lib.friends.set_following(following);
        let generation = core.lib.friends.generation();
        if self.listen.friends_checking || (!following && self.listen.friends_generation == Some(generation)) {
            return;
        }
        self.refresh_friends(cx);
    }

    /// Reads the friends now, and takes one step while following one.
    fn refresh_friends(&mut self, cx: &mut Context<Self>) {
        let Some(core) = self.core.as_ref() else { return };
        if self.listen.friends_checking {
            return;
        }
        self.listen.friends_checking = true;
        let lib = core.lib.clone();
        let core_generation = self.core_generation;
        cx.spawn(async move |this, cx| {
            // Read the generation first, so a change landing during the read is picked up next tick.
            let (generation, friends) = cx.background_executor().spawn(async move { (lib.friends.generation(), lib.spotify_friends()) }).await;
            let _ = this.update(cx, |shell, cx| {
                shell.listen.friends_checking = false;
                if shell.core_generation != core_generation {
                    return;
                }
                let changed = shell.listen.friends != friends;
                shell.listen.friends = friends;
                shell.listen.friends_generation = Some(generation);
                if let Some(f) = shell.listen.following.as_ref().filter(|f| ly::friend_of(&f.followed.player).is_some()) {
                    let local = shell.local_along(f);
                    let step = ly::friend_step(&f.followed, &shell.listen.friends, &local, ly::now_ms());
                    shell.apply_follow_step(step, cx);
                }
                if changed {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Where the listener's own song is, to resume when following ends. Switching from following
    /// someone else keeps the position saved then.
    fn own_position_ms(&mut self) -> u64 {
        if let Some(prior) = self.listen.following.take() {
            return prior.back_ms;
        }
        if self.player.station.is_some() { 0 } else { (self.player.position.max(0.0) * 1000.0) as u64 }
    }

    /// The listener's player as following sees it.
    fn local_along(&self, f: &Following) -> la::Local {
        la::Local {
            song_id: f.song.song.id.clone(),
            position_ms: (self.player.position.max(0.0) * 1000.0) as u64,
            playing: self.player.playing,
            held: f.held,
            own_change_ms: f.own_change_ms,
            seen: f.seen,
        }
    }

    /// Asks the server now. The list refreshes every 15 s, every 5 s while following, and at once
    /// when the followed song ends. When following a Spotify friend, reads the friends instead.
    pub fn check_listening(&mut self, cx: &mut Context<Self>) {
        if self.following_friend().is_some() {
            self.refresh_friends(cx);
            return;
        }
        let Some(core) = self.core.as_ref() else { return };
        if self.listen.checking {
            return;
        }
        self.listen.checking = true;
        let lib = core.lib.clone();
        let generation = self.core_generation;
        cx.spawn(async move |this, cx| {
            let found = cx.background_executor().spawn(async move { lib.now_playing() }).await;
            let _ = this.update(cx, |shell, cx| {
                shell.listen.checking = false;
                if shell.core_generation != generation {
                    return;
                }
                // Poll often while listeners may control playback, to notice them join and act.
                let every = if shell.listen.following.is_some() || shell.listeners_control() { la::FOLLOW_POLL } else { la::LIST_POLL };
                shell.listen.next_check = Some(Instant::now() + every);
                if let Some(entries) = found {
                    shell.on_now_playing(entries, cx);
                }
            });
        })
        .detach();
    }

    fn on_now_playing(&mut self, entries: Vec<NowPlaying>, cx: &mut Context<Self>) {
        let viewer = self.core.as_ref().map(|c| c.lib.username().to_string()).unwrap_or_default();
        self.listen.listing = la::people(&entries, &viewer, self.listen_active());
        self.listen.listing_at = Some(Instant::now());
        if let Some(f) = self.listen.following.as_ref().filter(|f| ly::friend_of(&f.followed.player).is_none()) {
            let step = la::follow_step(&f.followed, &entries, &self.local_along(f));
            let seen = la::followed_entry(&f.followed, &entries).map(|e| (e.state.is_some_and(la::SessionState::is_playing), ly::now_ms()));
            self.apply_follow_step(step, cx);
            if let Some(f) = self.listen.following.as_mut() {
                f.seen = seen;
            }
        }
        self.host_check(&entries, cx);
        self.apply_listen_marks();
        cx.notify();
    }

    /// Applies what one follow check asks of the player.
    fn apply_follow_step(&mut self, step: FollowStep, cx: &mut Context<Self>) {
        match step {
            FollowStep::Ended => self.listen_gone(cx),
            FollowStep::Keep { player, .. } => {
                if let Some(f) = self.listen.following.as_mut() {
                    f.followed.player = player;
                }
            }
            FollowStep::Resync { entry, player } => self.follow_to(entry, player, cx),
            FollowStep::NotInLibrary { key, song, artist } => self.skip_missing(key, &song, &artist, cx),
        }
    }

    /// Handles a followed friend playing a song that isn't in the library. The player pauses, a
    /// toast says so once, and following continues to their next song.
    fn skip_missing(&mut self, key: String, song: &str, artist: &str, cx: &mut Context<Self>) {
        if self.player.playing
            && let Some(core) = self.core.as_ref()
        {
            core.player.pause();
            self.player.playing = false;
            self.player.status = PlayerStatus::Paused;
        }
        if let Some(f) = self.listen.following.as_mut() {
            f.waiting = true;
        }
        if self.listen.skipped.as_deref() != Some(key.as_str()) {
            self.listen.skipped = Some(key);
            self.show_toast(trf("“{song}” by {artist} isn’t in your library, so it was skipped.", &[("song", song), ("artist", artist)]), cx);
        }
        cx.notify();
    }

    /// Starts following a Spotify friend from the confirm dialog, at their position if the song is
    /// in the library. Otherwise following starts waiting, with the listener's song paused, until
    /// they play a song that is.
    pub fn listen_along_friend(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(friend) = self.listen.friends.iter().find(|f| f.friend.id == id).cloned() else { return };
        let Some(track) = friend.track.clone() else { return };
        self.listen.skipped = None;
        if let Some(entry) = friend.session(ly::now_ms()) {
            self.listen_along(entry, cx);
            return;
        }
        if self.core.is_none() {
            return;
        }
        self.listen.ended = None;
        let back_ms = self.own_position_ms();
        let song = navia_core::client::Song { title: track.song.clone(), ..Default::default() };
        let entry = NowPlaying { user: friend.display_name(), player: ly::friend_player(id), song, state: Some(la::SessionState::Playing), position_ms: Some(0), minutes_ago: 0 };
        let followed = Followed { user: entry.user.clone(), player: entry.player.clone() };
        self.listen.following = Some(Following { followed, song: entry, rowid: None, back_ms, waiting: true, held: false, own_change_ms: None, seen: None });
        // The confirm already said this song is missing, so the chip shows the wait without a toast.
        self.listen.skipped = Some(track.key());
        self.skip_missing(track.key(), &track.song, track.first_artist(), cx);
    }

    /// Starts following `entry`'s user after the confirm. Their song plays here from the same
    /// moment, in place of the queue.
    pub fn listen_along(&mut self, entry: NowPlaying, cx: &mut Context<Self>) {
        if !entry.can_listen_along() || self.core.is_none() {
            return;
        }
        self.listen.ended = None;
        // The listener's own song resumes from here when following ends, or from the earlier spot
        // when switching from following someone else.
        let back_ms = self.own_position_ms();
        if self.player.station.is_some() {
            self.player.station = None;
        }
        let followed = Followed { user: entry.user.clone(), player: entry.player.clone() };
        self.listen.following = Some(Following { followed, song: entry.clone(), rowid: None, back_ms, waiting: false, held: false, own_change_ms: None, seen: None });
        self.load_followed(&entry, cx);
        self.listen.next_check = Some(Instant::now() + la::FOLLOW_POLL);
    }

    /// Follows a change found by a check. A new song, a pause or resume, or too much drift.
    fn follow_to(&mut self, entry: NowPlaying, player: String, cx: &mut Context<Self>) {
        let Some(f) = self.listen.following.as_mut() else { return };
        f.followed.player = player;
        f.waiting = false;
        f.held = false;
        let same_song = f.song.song.id == entry.song.id;
        f.song = entry.clone();
        let Some(core) = self.core.as_ref() else { return };
        let at = entry.position_ms.unwrap_or(0);
        let playing = entry.state.is_some_and(la::SessionState::is_playing);
        if !same_song {
            self.load_followed(&entry, cx);
            return;
        }
        // The same song, so catch up and match play or pause. These go straight to the player, not
        // through the controls that would end following.
        if ((self.player.position.max(0.0) * 1000.0) as u64).abs_diff(at) > la::RESYNC_DRIFT.as_millis() as u64 {
            core.player.seek_ms(at);
            self.player.position = at as f32 / 1000.0;
        }
        if playing != self.player.playing {
            if playing { core.player.play() } else { core.player.pause() }
        }
    }

    /// Plays the followed song alone in the player. The listener's queue stays as it is.
    fn load_followed(&mut self, entry: &NowPlaying, cx: &mut Context<Self>) {
        let Some(core) = self.core.as_mut() else { return };
        let rowid = core.lib.rowid_for_song(&entry.song.id);
        let duration = entry.song.duration.map(|d| d.max(0) as u64 * 1000);
        let at = entry.position_ms.unwrap_or(0);
        let playing = entry.state.is_some_and(la::SessionState::is_playing);
        // Their songs cut in as they change, without a crossfade.
        core.player.set_following(true);
        core.player.set_queue(vec![QueueItem::new(entry.song.id.clone(), duration)], 0, at, !playing);
        let song = crate::core_library::song_from_core(entry.song.clone());
        if let Some(f) = self.listen.following.as_mut() {
            f.rowid = rowid;
        }
        self.player.current = Some(song);
        self.player.song_ix = Some(rowid.map_or(crate::core_player::LIVE_ROW, |r| r as usize));
        self.player.position = at as f32 / 1000.0;
        self.player.playing = playing;
        self.player.status = if playing { PlayerStatus::Buffering } else { PlayerStatus::Paused };
        self.lyrics.clear();
        self.lyrics_song = None;
        self.request_tint(cx);
        self.publish_integrations(true);
        cx.notify();
    }

    /// The listener took control, so following ends with a toast. The followed song stays in the
    /// queue just before their own song. Returns true when following was on.
    pub fn take_control_from_along(&mut self, cx: &mut Context<Self>) -> bool {
        self.stop_listening(true, cx)
    }

    /// Something else starts playing, so following ends with a toast and the followed song isn't kept.
    pub fn drop_along(&mut self, cx: &mut Context<Self>) {
        let Some(f) = self.listen.following.take() else { return };
        if let Some(core) = self.core.as_ref() {
            core.player.set_following(false);
        }
        self.listen.resume = None;
        self.show_toast(trf("Stopped listening along with {user}", &[("user", &f.followed.user)]), cx);
    }

    /// They stopped or went quiet. The song stays, paused, and the player says so for a moment.
    fn listen_gone(&mut self, cx: &mut Context<Self>) {
        let Some(user) = self.listen.following.as_ref().map(|f| f.followed.user.clone()) else { return };
        self.settle_followed(true, cx);
        self.listen.ended = Some(user);
        self.listen.ended_generation += 1;
        let generation = self.listen.ended_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(la::ENDED_NOTICE).await;
            let _ = this.update(cx, |shell, cx| {
                if shell.listen.ended_generation == generation {
                    shell.listen.ended = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The X on "<user> stopped listening".
    pub fn dismiss_listen_ended(&mut self, cx: &mut Context<Self>) {
        self.listen.ended = None;
        cx.notify();
    }

    /// Ends following. `toast` is set when the listener took control. Returns whether following was on.
    pub fn stop_listening(&mut self, toast: bool, cx: &mut Context<Self>) -> bool {
        let Some(user) = self.listen.following.as_ref().map(|f| f.followed.user.clone()) else { return false };
        self.settle_followed(false, cx);
        if toast {
            self.show_toast(trf("Stopped listening along with {user}", &[("user", &user)]), cx);
        }
        true
    }

    /// Makes the followed song the current queue entry, just before the listener's own song, which
    /// resumes where it was. The player keeps playing without a break. `pause` stops it there.
    fn settle_followed(&mut self, pause: bool, cx: &mut Context<Self>) {
        let Some(f) = self.listen.following.take() else { return };
        let Some(core) = self.core.as_ref() else { return };
        let player = core.player.clone();
        // The listener's own queue crossfades again. The followed song still ends with a cut.
        player.set_following(false);
        if pause {
            player.pause();
            self.player.playing = false;
            self.player.status = PlayerStatus::Paused;
        }
        let Some(rowid) = f.rowid else {
            // Not in this library, so the queue comes back as it was, paused at its song.
            self.listen.resume = None;
            if !self.player.queue.is_empty() {
                let at = self.player.queue_pos.min(self.player.queue.len() - 1);
                let edits = core.pq.load_edits();
                self.queue_edits_at(edits, f.back_ms, true);
                self.player.song_ix = Some(self.player.queue[at] as usize);
                self.player.current = self.song_peek(self.player.queue[at]);
                self.player.position = f.back_ms as f32 / 1000.0;
                self.player.playing = false;
                self.player.status = PlayerStatus::Paused;
            }
            cx.notify();
            return;
        };
        let pos = self.player.queue_pos.min(self.player.queue.len());
        let had_own = pos < self.player.queue.len();
        let own = self.player.queue.get(pos).copied();
        // The player holds only the followed song, so the queue is restored around it. Sent now, in
        // order, so a skip or seek that ended following reaches the player afterward.
        let Some(core) = self.core.as_mut() else { return };
        let edits = core.pq.keep_in_front(rowid);
        self.queue_edits(edits);
        self.player.song_ix = Some(rowid as usize);
        self.listen.resume = had_own.then(|| (own.unwrap_or_default(), pos + 1, f.back_ms));
        self.core_save_session();
        cx.notify();
    }

    /// Resumes the listener's own song from where it was after following.
    pub fn resume_after_along(&mut self, index: usize, rowid: u32) {
        let Some((row, at, ms)) = self.listen.resume else { return };
        if index != at || row != rowid {
            self.listen.resume = None;
            return;
        }
        self.listen.resume = None;
        if ms > 0
            && let Some(core) = self.core.as_ref()
        {
            core.player.seek_ms(ms);
            self.player.position = ms as f32 / 1000.0;
        }
    }

    /// Whether `user` on this server is followed now.
    pub fn following_user(&self, user: &str) -> bool {
        self.listen.following.as_ref().is_some_and(|f| ly::friend_of(&f.followed.player).is_none() && f.followed.user.eq_ignore_ascii_case(user))
    }
}
