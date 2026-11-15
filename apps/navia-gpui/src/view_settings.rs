//! Settings. Controls without a backend yet are shown disabled with the reason.

use crate::i18n::{self, tr, trf, trn, n_};
use std::rc::Rc;

use gpui::prelude::*;
use crate::focus::FocusStop;
use gpui::{AnyElement, Context, FontWeight, Hsla, MouseButton, SharedString, Window, div, px, relative};
use navia_core::audio::player::Player as CorePlayer;
use navia_core::quality::{Codec, Leveling, Quality, QualityControl, offered_codecs};
use navia_core::reporting::PlayThreshold;

use crate::color::{hex_to_hue, hue_to_hex, is_hex, parse_hex};
use crate::eq_model::OUTPUT_DEFAULT;

/// The GPL text Settings > About links to.
pub const LICENSE_URL: &str = "https://www.gnu.org/licenses/gpl-3.0.html";

/// The output pickers in Settings and the Equalizer, refreshed when devices change.
pub const OUTPUT_SELECT: &str = "output-device";
pub const EQ_OUTPUT_SELECT: &str = "eq-output";
use crate::persist::{BetweenSongs, DiscordArt, PlayerMode};
use crate::shell::{Layout, Route, Shell};
use crate::theme::Theme;
use crate::theme::size::*;
use crate::ui::{glass_edge, icon};
use crate::ui_controls::{SelectOption, SliderLook, SliderSpec, TextTarget, ring, segmented_settings, switch};

pub const ACCENT_PRESETS: [u32; 7] = [0x5a93f5, 0x50ffff, 0x8b7cf6, 0xe879a6, 0xf2685c, 0xf5a33d, 0x4cc38a];
const ZOOM_MIN: f32 = 70.0;
const ZOOM_MAX: f32 = 130.0;
const ZOOM_STEP: f32 = 5.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RowPos {
    First,
    Middle,
    Last,
    Single,
}

pub fn section_title(t: &Theme, title: &'static str) -> gpui::Div {
    div().mb(t.s(10.0)).text_size(t.text(TEXT_SECTION)).font_weight(FontWeight::SEMIBOLD).child(title)
}

fn row_shell(t: &Theme, pos: RowPos, main_tokens: f32) -> gpui::Div {
    let r = t.s(14.0);
    let mut row = div().flex().flex_row().items_center().justify_between().gap(t.s(24.0)).min_h(t.s(60.0)).px(t.s(16.0)).py(t.s(10.0)).bg(t.panel_glass());
    if main_tokens <= 620.0 {
        row = row.flex_wrap();
    }
    row = match pos {
        RowPos::First => row.rounded_tl(r).rounded_tr(r).border_b_1().border_color(t.line()),
        RowPos::Middle => row.border_b_1().border_color(t.line()),
        RowPos::Last => row.rounded_bl(r).rounded_br(r),
        RowPos::Single => row.rounded(r),
    };
    row
}

fn row_text(t: &Theme, label: impl Into<SharedString>, hint: Option<SharedString>, dim: bool) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .line_height(relative(1.35))
        .child(div().font_weight(FontWeight::MEDIUM).when(dim, |d| d.text_color(t.t2())).child(label.into()))
        .when_some(hint, |d, hint| d.child(div().mt(t.s(1.0)).text_size(t.text(TEXT_SM)).text_color(t.t2()).child(hint)))
}

/// A Settings row's text button, dimmed when it can't act.
pub fn settings_button(t: &Theme, id: impl Into<gpui::ElementId>, label: impl Into<SharedString>, enabled: bool) -> gpui::Stateful<gpui::Div> {
    div().id(id).flex().items_center().flex_none().h(t.s(34.0)).px(t.s(14.0)).rounded_full().bg(t.panel_2()).text_size(t.text(TEXT_SM)).font_weight(FontWeight::MEDIUM).when(!enabled, |d| d.opacity(DISABLED_OPACITY)).when(enabled, |d| d.child(crate::ui::hover_pill(t))).child(label.into())
}

pub fn settings_row(t: &Theme, label: impl Into<SharedString>, hint: Option<SharedString>, control: AnyElement, pos: RowPos, main_tokens: f32) -> gpui::Div {
    row_shell(t, pos, main_tokens).child(row_text(t, label, hint, false)).child(control)
}

/// A row that opens a page, with label, hint, an optional value and a caret. Hover fill is its only feedback.
pub(crate) fn settings_link_row(t: &Theme, id: &'static str, label: impl Into<SharedString>, hint: SharedString, value: Option<AnyElement>, pos: RowPos, main_tokens: f32) -> gpui::Stateful<gpui::Div> {
    row_shell(t, pos, main_tokens)
        .id(id)
        .hover(|s| s.bg(t.hover()))
        .child(row_text(t, label, Some(hint), false))
        .child(div().flex().flex_row().flex_none().items_center().gap(t.s(10.0)).children(value).child(icon(t, "caret-right-bold", 16.0, t.t3())))
}

impl Shell {
    /// The "Use Deezer for recommendations" row. This account's choice alone decides Navia's own Deezer requests, so
    /// it's never dimmed.
    fn render_deezer_row(&mut self, main_tokens: f32, cx: &mut Context<Self>) -> gpui::Div {
        let t = self.theme;
        let on = self.deezer.choice == Some(true);
        let control = switch(&t, "deezer", on, true, cx, move |this, cx| this.set_deezer_choice(!on, cx)).into_any_element();
        let hint = tr("Better Autoplay and Artist Radio. Sends artist and song names to Deezer from this computer. No account needed.");
        row_shell(&t, RowPos::Middle, main_tokens).child(row_text(&t, tr("Use Deezer for recommendations"), Some(hint.into()), false)).child(control)
    }
}

/// A control with no backend yet. It's dimmed and its row says why.
fn disabled_pill(t: &Theme, value: impl Into<SharedString>) -> AnyElement {
    div().flex().flex_row().items_center().justify_between().flex_none().min_w(t.s(240.0)).max_w_full().h(t.s(CONTROL_H)).pl(t.s(16.0)).pr(t.s(12.0)).rounded_full().bg(t.glass_fill()).shadow(glass_edge()).opacity(0.4).child(div().truncate().child(value.into())).child(icon(t, "caret-up-down-bold", 14.0, t.t2())).into_any_element()
}

const STREAM_QUALITY_HINT: &str = n_("Original plays the files as they are, bit for bit. Other formats make the server convert each song, using its CPU, and aren't bit-exact. MP3 and AAC can leave up to about 25 ms between gapless tracks; Opus doesn't. If a format isn't available, Navia plays the original.");
const DOWNLOAD_QUALITY_HINT: &str = n_("Original keeps the files as they are. Other formats make the server convert each song once, using its CPU. Songs you've already downloaded stay as they are.");
const LEVELING_HINT: &str = n_("Uses ReplayGain from the files, limited by their peak so it can't clip. Off plays every file unchanged. On a transcoded stream the peak is the original's, so levels can overshoot slightly.");
/// The hint under Settings > Spotify Friends.
const FRIENDS_HINT: &str = n_("Follow friends who listen on Spotify and hear the same songs from your library. For their Spotify activity to be readable, they need to join Lanyard’s Discord server.");
const FRIEND_ID_HINT: &str = n_("Their Discord user ID. In Discord, turn on Developer Mode, then right-click them and choose Copy User ID.");
/// The hint under Settings > Let listeners control playback.
const CONTROL_HINT: &str = n_("People listening along with you can pause, skip, seek and change the song.");
/// The hint under Settings > Share what I'm playing.
const SHARE_HINT: &str = n_("Others using Navia on this server can see what you play and listen along, and you can see who else shares.");
/// The hints for the two Autoplay modes.
const AUTOPLAY_NAVIA_HINT: &str = n_("Picks from the server's suggestions, your plays and favorites, and Deezer when it's on.");
const AUTOPLAY_SERVER_HINT: &str = n_("The server's similar songs, as they come.");
const NO_FORMAT: &str = n_("The server can't make this format.");

/// Which saved quality a row edits.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum QualitySlot {
    Streaming,
    Metered,
    Download,
}

impl QualitySlot {
    fn get(self, control: &QualityControl) -> Quality {
        match self {
            QualitySlot::Streaming => control.streaming(),
            QualitySlot::Metered => control.metered_streaming(),
            QualitySlot::Download => control.download(),
        }
    }

    fn set(self, control: &QualityControl, quality: Quality) {
        let saved = match self {
            QualitySlot::Streaming => control.set_streaming(quality),
            QualitySlot::Metered => control.set_metered_streaming(quality),
            QualitySlot::Download => control.set_download(quality),
        };
        if let Err(e) = saved {
            eprintln!("navia: couldn't save the quality setting: {e}");
        }
    }

    fn key(self, part: &str) -> String {
        let slot = match self {
            QualitySlot::Streaming => "streaming",
            QualitySlot::Metered => "metered",
            QualitySlot::Download => "download",
        };
        format!("quality-{slot}-{part}")
    }
}

impl Shell {
    fn quality_control(&self) -> Option<std::sync::Arc<QualityControl>> {
        self.core.as_ref().map(|c| c.lib.downloads.quality_control())
    }

    fn metered_hint(&self) -> SharedString {
        match self.quality_control().map(|c| c.is_metered()) {
            Some(true) => tr("Used instead of Streaming quality when your system marks the connection as metered, like a phone hotspot. This connection counts as metered."),
            Some(false) => tr("Used instead of Streaming quality when your system marks the connection as metered, like a phone hotspot. This connection doesn't count as metered."),
            None => tr("Used instead of Streaming quality when your system marks the connection as metered, like a phone hotspot. Applies when your system says the connection is metered."),
        }.into()
    }

    /// Checks once in the background which formats the server refuses, the first time Settings opens.
    fn ensure_format_check(&mut self, cx: &mut Context<Self>) {
        if self.format_check_started || self.core.is_none() {
            return;
        }
        self.format_check_started = true;
        let Some(control) = self.quality_control() else { return };
        cx.spawn(async move |this, cx| {
            let found = cx.background_spawn(async move { control.detect_formats() }).await;
            let _ = this.update(cx, |this, cx| {
                this.unavailable_formats = found.unwrap_or_default().into_iter().filter(|(_, can)| !can).map(|(c, _)| c).collect();
                cx.notify();
            });
        })
        .detach();
    }

    fn render_quality_row(&mut self, label: &'static str, hint: SharedString, slot: QualitySlot, main_tokens: f32, cx: &mut Context<Self>) -> gpui::Div {
        self.render_quality_row_at(label, hint, slot, RowPos::Middle, main_tokens, cx)
    }

    fn render_quality_row_at(&mut self, label: &'static str, hint: SharedString, slot: QualitySlot, pos: RowPos, main_tokens: f32, cx: &mut Context<Self>) -> gpui::Div {
        let t = self.theme;
        self.ensure_format_check(cx);
        let Some(control) = self.quality_control() else {
            return row_shell(&t, pos, main_tokens).child(row_text(&t, label, Some(tr("Needs a connection to the server.").into()), true)).child(disabled_pill(&t, tr("Original")));
        };
        let quality = slot.get(&control);
        let mut formats = vec![SelectOption::new("original", tr("Original"))];
        for codec in offered_codecs() {
            formats.push(if self.unavailable_formats.contains(&codec) { SelectOption::unavailable(codec.id(), codec.label(), tr(NO_FORMAT)) } else { SelectOption::new(codec.id(), codec.label()) });
        }
        let format_picker = self.select_pill(&slot.key("format"), quality.format_id(), Rc::new(formats), tr("Choose"), Some(140.0), move |this, value, cx| {
            let Some(control) = this.quality_control() else { return };
            let chosen = Codec::from_id(&value).map_or(Quality::Original, |c| Quality::Transcode { codec: c, kbps: c.default_kbps() });
            slot.set(&control, chosen);
            cx.notify();
        }, cx);
        let rate_picker: AnyElement = match quality {
            Quality::Original => div().flex().flex_row().items_center().justify_between().flex_none().min_w(t.s(140.0)).max_w_full().h(t.s(CONTROL_H)).pl(t.s(16.0)).pr(t.s(12.0)).rounded_full().bg(t.glass_fill()).shadow(glass_edge()).opacity(0.4).child(div().truncate().child(tr("Source"))).child(icon(&t, "caret-up-down-bold", 14.0, t.t2())).into_any_element(),
            Quality::Transcode { codec, kbps } => {
                let rates = codec.bitrates().iter().map(|r| SelectOption::new(r.to_string(), format!("{r} kbps"))).collect();
                self.select_pill(&slot.key("rate"), kbps.to_string(), Rc::new(rates), tr("Choose"), Some(140.0), move |this, value, cx| {
                    let Some(control) = this.quality_control() else { return };
                    let Ok(kbps) = value.parse() else { return };
                    slot.set(&control, Quality::Transcode { codec, kbps });
                    cx.notify();
                }, cx)
            }
        };
        let pair = div().flex().flex_row().flex_none().gap(t.s(8.0)).child(format_picker).child(rate_picker).into_any_element();
        settings_row(&t, label, Some(hint), pair, pos, main_tokens)
    }

    fn render_leveling_row(&mut self, main_tokens: f32, cx: &mut Context<Self>) -> gpui::Div {
        let t = self.theme;
        let Some(control) = self.quality_control() else {
            return row_shell(&t, RowPos::Middle, main_tokens).child(row_text(&t, tr("Volume leveling"), Some(tr("Needs a connection to the server.").into()), true)).child(disabled_pill(&t, tr("Off")));
        };
        let options = Rc::new(vec![SelectOption::new("off", tr("Off")), SelectOption::new("album", tr("Album")), SelectOption::new("track", tr("Track"))]);
        let picker = self.select_pill("volume-leveling", control.leveling().id(), options, tr("Choose"), Some(240.0), |this, value, cx| {
            if let (Some(control), Some(mode)) = (this.quality_control(), Leveling::from_id(&value))
                && let Err(e) = control.set_leveling(mode)
            {
                eprintln!("navia: couldn't save volume leveling: {e}");
            }
            cx.notify();
        }, cx);
        settings_row(&t, tr("Volume leveling"), Some(tr(LEVELING_HINT).into()), picker, RowPos::Middle, main_tokens)
    }

    pub fn build_settings(&mut self, _layout: &Layout) -> Vec<crate::pages::Block> {
        let section = |render: fn(&mut Shell, &Layout, &mut Window, &mut Context<Shell>) -> AnyElement| Shell::plain_block(move |this, layout, window, cx| render(this, layout, window, cx));
        vec![
            section(Self::render_settings_playback),
            crate::pages::Block::Spacer(30.0),
            section(Self::render_settings_appearance),
            crate::pages::Block::Spacer(30.0),
            section(Self::render_settings_player),
            crate::pages::Block::Spacer(30.0),
            section(Self::render_settings_startup),
            crate::pages::Block::Spacer(30.0),
            section(Self::render_settings_library),
            crate::pages::Block::Spacer(30.0),
            section(Self::render_settings_storage),
            crate::pages::Block::Spacer(30.0),
            section(Self::render_settings_connections),
            crate::pages::Block::Spacer(30.0),
            section(Self::render_settings_friends),
            crate::pages::Block::Spacer(30.0),
            section(Self::render_settings_about),
        ]
    }

    fn settings_frame(&self, layout: &Layout, content: gpui::Div) -> AnyElement {
        let t = self.theme;
        div().w(layout.main_width).child(div().w_full().max_w(t.s(780.0)).mx_auto().px(t.s(GUTTER)).child(content)).into_any_element()
    }

    pub fn ensure_outputs(&mut self, cx: &mut Context<Self>) {
        if self.outputs_requested || self.library.is_none() {
            return;
        }
        self.outputs_requested = true;
        if self.core.is_none() {
            if self.sample_mode {
                self.outputs = vec![("speakers".to_string(), tr("Sample speakers").to_string()), ("headphones".to_string(), tr("Sample headphones").to_string())];
            }
            return;
        }
        cx.spawn(async move |this, cx| {
            let list = cx.background_spawn(async { CorePlayer::list_devices().unwrap_or_default() }).await;
            let _ = this.update(cx, |this, cx| {
                this.outputs = list.into_iter().map(|d| (d.id, d.name)).collect();
                // A menu or picker opened before the list arrived, or across a hot-plug.
                this.refresh_output_menu(cx);
                let options = this.output_options();
                this.refresh_select(OUTPUT_SELECT, options.clone(), cx);
                this.refresh_select(EQ_OUTPUT_SELECT, options, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Keeps the device list current as devices come and go and the system default changes. Only where the core can
    /// watch, which is a Linux sound server.
    pub fn watch_outputs(&mut self, cx: &mut Context<Self>) {
        // List them at startup so the first menu already has them.
        self.ensure_outputs(cx);
        if self.output_watch.is_some() {
            return;
        }
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<()>();
        self.output_watch = navia_core::audio::device_watch::watch(move || {
            let _ = tx.unbounded_send(());
        });
        if self.output_watch.is_none() {
            return;
        }
        cx.spawn(async move |this, cx| {
            use futures::StreamExt;
            while rx.next().await.is_some() {
                let refreshed = this.update(cx, |this, cx| {
                    this.outputs_requested = false;
                    this.ensure_outputs(cx);
                });
                if refreshed.is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    pub fn output_label(&self, id: &str) -> String {
        if id == OUTPUT_DEFAULT {
            return tr("System default").to_string();
        }
        self.outputs.iter().find(|(d, _)| d == id).map_or_else(|| id.to_string(), |(_, n)| n.clone())
    }

    pub fn output_options(&self) -> Rc<Vec<SelectOption>> {
        let mut options = vec![SelectOption::new(OUTPUT_DEFAULT, tr("System default"))];
        options.extend(self.outputs.iter().map(|(id, name)| SelectOption::new(id.clone(), name.clone())));
        Rc::new(options)
    }

    pub fn set_output_device(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        self.airplay_output_changed();
        self.prefs.output_device = id.clone();
        self.prefs.save();
        if let Some(core) = self.core.as_ref() {
            core.player.set_output_device(id);
        }
        cx.notify();
    }

    /// Applies the saved playback settings to a newly attached player.
    pub fn apply_playback_settings(&self) {
        let Some(core) = self.core.as_ref() else { return };
        core.player.set_crossfade_ms(self.prefs.crossfade_ms());
        core.player.set_smart_crossfade(self.prefs.between_songs == BetweenSongs::Smart);
        if self.prefs.output_device.is_some() {
            core.player.set_output_device(self.prefs.output_device.clone());
        }
    }

    fn set_between_songs(&mut self, style: BetweenSongs, cx: &mut Context<Self>) {
        self.prefs.between_songs = style;
        self.prefs.save();
        if let Some(core) = self.core.as_ref() {
            core.player.set_crossfade_ms(self.prefs.crossfade_ms());
            core.player.set_smart_crossfade(style == BetweenSongs::Smart);
        }
        cx.notify();
    }

    fn render_settings_playback(&mut self, layout: &Layout, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        self.ensure_outputs(cx);
        let main_tokens = f32::from(layout.main_width) / t.token_scale();
        let style = self.prefs.between_songs;
        let segmented = segmented_settings(&t, "between-songs", style, &[(BetweenSongs::Gapless, tr("Gapless"), None), (BetweenSongs::Crossfade, tr("Crossfade"), None), (BetweenSongs::Smart, tr("Smart"), None)], cx, |this, value, cx| this.set_between_songs(value, cx)).into_any_element();
        let hint: SharedString = tr("Gapless keeps albums seamless. Smart crossfades between unrelated songs, and stays gapless within an album.").into();
        let mut col = div().flex().flex_col().child(section_title(&t, tr("Playback"))).child(settings_row(&t, tr("Between songs"), Some(hint), segmented, RowPos::First, main_tokens));
        if style != BetweenSongs::Gapless {
            let fade = self.prefs.crossfade_s as f32;
            let slider = self.render_slider(SliderSpec::new("crossfade-length", fade, 1.0, 12.0, |this, v, cx| {
                this.prefs.crossfade_s = v.round() as u32;
                if let Some(core) = this.core.as_ref() {
                    core.player.set_crossfade_ms(this.prefs.crossfade_ms());
                }
                cx.notify();
            }).look(SliderLook::Plain).on_end(|this, _| this.prefs.save()), cx);
            col = col.child(settings_row(&t, tr("Crossfade length"), Some(trn("{count} second", "{count} seconds", self.prefs.crossfade_s as usize).into()), slider, RowPos::Middle, main_tokens));
        }
        use navia_core::queue_order::ShuffleStyle;
        let shuffle_style = self.prefs.shuffle_style;
        let shuffle = segmented_settings(&t, "shuffle-style", shuffle_style, &[(ShuffleStyle::FewerRepeats, tr("Fewer Repeats"), None), (ShuffleStyle::Standard, tr("Standard"), None)], cx, |this, value, cx| {
            this.prefs.shuffle_style = value;
            this.prefs.save();
            cx.notify();
        })
        .into_any_element();
        let shuffle_hint = if shuffle_style == ShuffleStyle::FewerRepeats { tr("Fewer Repeats plays songs you heard recently later.") } else { tr("Standard plays the list in a plain random order.") };
        col = col.child(settings_row(&t, tr("Shuffle"), Some(shuffle_hint.into()), shuffle, RowPos::Middle, main_tokens));
        let autoplay = switch(&t, "autoplay", self.prefs.autoplay, true, cx, |this, cx| {
            this.prefs.autoplay = !this.prefs.autoplay;
            this.prefs.save();
            this.autoplay_update(None, cx);
            cx.notify();
        })
        .into_any_element();
        col = col
            .child(settings_row(&t, tr("Autoplay"), Some(tr("When your music ends, keep playing similar songs. Starts only once nothing is left, so after every song of a shuffled list, and never while Repeat is on.").into()), autoplay, RowPos::Middle, main_tokens));
        if self.prefs.autoplay {
            use navia_core::engine::AutoplayMode;
            let mode = self.autoplay_mode;
            let picker = segmented_settings(&t, "autoplay-mode", mode, &[(AutoplayMode::Navia, tr("Navia"), None), (AutoplayMode::Server, tr("Server"), None)], cx, |this, value, cx| this.set_autoplay_mode(value, cx)).into_any_element();
            let hint = if mode == AutoplayMode::Navia { tr(AUTOPLAY_NAVIA_HINT) } else { tr(AUTOPLAY_SERVER_HINT) };
            col = col.child(settings_row(&t, tr("Autoplay mode"), Some(hint.into()), picker, RowPos::Middle, main_tokens));
        }
        col = col
            .child(self.render_deezer_row(main_tokens, cx))
            .child(self.render_leveling_row(main_tokens, cx))
            .child(self.render_quality_row(tr("Streaming quality"), tr(STREAM_QUALITY_HINT).into(), QualitySlot::Streaming, main_tokens, cx))
            .child(self.render_quality_row(tr("On metered connections"), self.metered_hint(), QualitySlot::Metered, main_tokens, cx));
        let output_value = self.prefs.output_device.clone().unwrap_or_else(|| OUTPUT_DEFAULT.to_string());
        let options = self.output_options();
        let picker = self.select_pill(OUTPUT_SELECT, output_value, options, tr("Choose"), Some(240.0), |this, value, cx| this.set_output_device((value != OUTPUT_DEFAULT).then(|| value.to_string()), cx), cx);
        let hint: SharedString = if self.outputs.is_empty() { tr("Your system's default output.").into() } else { tr("Where Navia plays. Applied to the player right away.").into() };
        col = col.child(settings_row(&t, tr("Output device"), Some(hint), picker, RowPos::Middle, main_tokens));
        let link = settings_link_row(&t, "equalizer-link", tr("Equalizer"), self.equalizer_summary().into(), None, RowPos::Last, main_tokens)
            .stop(&t)
            .on_click(cx.listener(|this, _, _, cx| this.navigate(Route::Equalizer, cx)));
        self.settings_frame(layout, col.child(link))
    }

    /// The "Public server address" field, as wide as the settings selects.
    fn render_discord_address(&mut self, cx: &mut Context<Self>) -> AnyElement {
        self.settings_field(TextTarget::DiscordAddress, "discord-address", "https://music.example.org", 240.0, cx)
    }

    /// The "Public server address" row. While empty and not being edited it shows whether the server's own address is
    /// used, plus a button to open the field. Hidden for External sources.
    fn render_discord_address_row(&mut self, main_tokens: f32, cx: &mut Context<Self>) -> Option<gpui::Div> {
        use navia_integrations::art::{CoverSource, PublicAddressError, parse_public_address};
        let t = self.theme;
        let source = self.prefs.discord_covers_from;
        if source == CoverSource::Search {
            return None;
        }
        let editing = self.focused_target() == Some(TextTarget::DiscordAddress) || !self.prefs.discord_public_address.trim().is_empty();
        if editing {
            let hint: SharedString = match parse_public_address(&self.prefs.discord_public_address) {
                Err(PublicAddressError::NotHttps) => tr("Only https addresses can be used.").into(),
                Err(_) => tr("Enter the address as https://music.example.org.").into(),
                Ok(_) => tr("If your server is reachable from the internet, Discord can show its own covers.").into(),
            };
            let field = self.render_discord_address(cx);
            return Some(settings_row(&t, tr("Public server address"), Some(hint), field, RowPos::Middle, main_tokens));
        }
        let using: SharedString = tr("Using your server's address for covers.").into();
        let status: Option<SharedString> = match source {
            CoverSource::Server => {
                let https = self.core.as_ref().is_some_and(|c| c.lib.server_url().trim_start().to_ascii_lowercase().starts_with("https://"));
                Some(if https { using } else { tr("Only https addresses can be used.").into() })
            }
            _ => self.server_address_public(cx).map(|public| if public { using } else { tr("Your server's address isn't public, so covers come from external sources.").into() }),
        };
        let button = crate::view_album::text_button_on(&t, "discord-address-open", None, tr("Use a different address"), true, t.t1().into())
            .on_click(cx.listener(|this, _, _, cx| this.focus_field(TextTarget::DiscordAddress, cx)))
            .into_any_element();
        Some(settings_row(&t, tr("Public server address"), status, button, RowPos::Middle, main_tokens))
    }

    fn disabled_row(&self, label: &'static str, reason: &'static str, control: AnyElement, main_tokens: f32) -> gpui::Div {
        let t = self.theme;
        row_shell(&t, RowPos::Middle, main_tokens).child(row_text(&t, label, Some(reason.into()), true)).child(control)
    }

    fn render_accent_picker(&mut self, main_tokens: f32, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let current = self.prefs.accent;
        if self.controls.text != Some(TextTarget::AccentHex) {
            self.controls.hex_text = format!("#{current:06x}");
        }
        let mut swatches = div().flex().flex_row().gap(t.s(7.0)).flex_none();
        for (i, preset) in ACCENT_PRESETS.into_iter().enumerate() {
            let on = preset == current;
            let color = gpui::rgb(preset);
            swatches = swatches.child(
                div()
                    .id(("accent-swatch", i))
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(t.s(26.0))
                    .rounded_full()
                    .bg(color)
                    .shadow(vec![crate::ui::inset_ring(gpui::rgba(crate::theme::GLASS_EDGE))])
                    .stop(&t).on_click(cx.listener(move |this, _, _, cx| {
                        this.set_accent(preset);
                        this.prefs.save();
                        cx.notify();
                    }))
                    .when(on, |d| d.child(div().absolute().left(t.s(-4.0)).top(t.s(-4.0)).right(t.s(-4.0)).bottom(t.s(-4.0)).rounded_full().border_2().border_color(t.t1())).child(icon(&t, "check-bold", 13.0, t.on_accent.hsla(1.0)))),
            );
        }
        let hue = self.render_slider(SliderSpec::new("accent-hue", hex_to_hue(current), 0.0, 359.0, |this, v, cx| {
            this.set_accent(hue_to_hex(v));
            cx.notify();
        }).look(SliderLook::Hue).width(Some(96.0)).on_end(|this, _| this.prefs.save()), cx);
        let bad = !is_hex(&self.controls.hex_text);
        let focused = self.controls.text == Some(TextTarget::AccentHex);
        let line = self.text_line(&TextTarget::AccentHex, &self.controls.hex_text.clone(), "", "accent-hex-text", window);
        let hex = div()
            .id("accent-hex")
            .flex()
            .flex_row()
            .items_center()
            .flex_none()
            .w(t.s(92.0)) // Inter clips the last digit of "#5a93f5" at 84.
            .h(t.s(34.0))
            .px(t.s(12.0))
            .rounded_full()
            .bg(t.panel_2())
            .text_size(t.text(TEXT_SM))
            .cursor_text()
            .when(focused, |d| d.shadow(vec![ring(t.accent_hsla())]))
            .when(bad, |d| d.shadow(vec![ring(Hsla::from(gpui::rgb(crate::theme::DANGER)))]))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                this.focus_text(TextTarget::AccentHex, cx);
            }))
            .child(line);
        div().flex().flex_row().items_center().flex_none().gap(t.s(12.0)).when(main_tokens <= 760.0, |d| d.flex_wrap()).child(swatches).child(hue).child(hex).into_any_element()
    }

    /// Settings actions for screenshots, like `accent:#e879a6`, `zoom:130`, `blur:0` and `crossfade`.
    pub fn settings_script(&mut self, op: &str, cx: &mut Context<Self>) -> bool {
        if let Some(hex) = op.strip_prefix("accent:") {
            if let Some(c) = parse_hex(hex) {
                self.set_accent(c);
                self.prefs.save();
            }
        } else if let Some(z) = op.strip_prefix("zoom:") {
            self.set_zoom(z.parse().unwrap_or(100), cx);
        } else if let Some(b) = op.strip_prefix("blur:") {
            self.prefs.lyric_blur = b == "1";
            self.prefs.save();
        } else if op == "crossfade" {
            self.set_between_songs(BetweenSongs::Crossfade, cx);
        } else if let Some(choice) = op.strip_prefix("login:") {
            self.set_open_at_login(crate::autostart::OpenAtLogin::from_id(choice), cx);
        } else if let Some(choice) = op.strip_prefix("close:") {
            self.set_close_to_tray(choice == "tray", cx);
        } else {
            return false;
        }
        cx.notify();
        true
    }

    pub fn set_accent(&mut self, accent: u32) {
        self.prefs.accent = accent;
        self.refresh_theme();
    }

    pub fn accent_hex_typed(&mut self, value: String, cx: &mut Context<Self>) {
        self.controls.hex_text = value.chars().take(7).collect();
        if is_hex(&self.controls.hex_text) {
            if let Some(c) = parse_hex(&self.controls.hex_text) {
                self.set_accent(c);
                self.prefs.save();
            }
        }
        cx.notify();
    }

    fn set_language(&mut self, code: &str, cx: &mut Context<Self>) {
        self.prefs.language = (code != "system").then(|| code.to_string());
        self.prefs.save();
        i18n::apply(self.prefs.language.as_deref());
        self.refresh_smart_options();
        self.invalidate_page();
        cx.notify();
    }

    fn render_settings_appearance(&mut self, layout: &Layout, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let main_tokens = f32::from(layout.main_width) / t.token_scale();
        let accent = self.render_accent_picker(main_tokens, window, cx);
        // Always the user's choice. Without detected blur it only starts off and says what to expect, since GNOME
        // extensions can blur windows without telling us.
        let on = self.translucent();
        let translucency = crate::ui_controls::switch_look(&t, on, 1.0)
            .id("translucency-switch")
            .stop(&t)
            .on_click(cx.listener(|this, _, window, cx| this.set_translucent(!this.translucent(), window, cx)))
            .into_any_element();
        let mut languages = vec![SelectOption::new("system", tr("System"))];
        languages.extend(i18n::LANGUAGE_NAMES.iter().map(|(code, name)| SelectOption::new(*code, *name)));
        let language = self.select_pill("language", self.prefs.language.clone().unwrap_or_else(|| "system".to_string()), Rc::new(languages), tr("Choose"), Some(240.0), |this, value, cx| this.set_language(value.as_ref(), cx), cx);
        let hint: SharedString = if self.blur_detected { tr("Lets your desktop show blurred behind the app. Needs a compositor that supports blur.").into() } else { tr("Your desktop may not blur windows, so translucency can look see-through.").into() };
        // Linux only, since Windows always uses Navia's frame. Where the desktop has no title bar to offer, as on
        // GNOME, the row is disabled and says why.
        let frame_row = cfg!(target_os = "linux").then(|| {
            let available = self.system_frame_available;
            let on = self.prefs.system_title_bar && available;
            let switch = switch(&t, "system-title-bar", on, available, cx, |this, cx| this.set_system_title_bar(!this.prefs.system_title_bar, cx)).into_any_element();
            let hint: SharedString = if available {
                tr("Shows your desktop's title bar and window buttons above Navia instead of Navia's own.").into()
            } else {
                tr("Not available on this desktop, because it doesn't provide a title bar.").into()
            };
            row_shell(&t, RowPos::Middle, main_tokens).child(row_text(&t, tr("Use the desktop's title bar"), Some(hint), !available)).child(switch)
        });
        let col = div()
            .flex()
            .flex_col()
            .child(section_title(&t, tr("Appearance")))
            .child(settings_row(&t, tr("Accent color"), None, accent, RowPos::First, main_tokens))
            .child(row_shell(&t, RowPos::Middle, main_tokens).child(row_text(&t, tr("Window translucency"), Some(hint), false)).child(translucency))
            .children(frame_row)
            .child(settings_row(&t, tr("Language"), Some(tr("Follows your system's language unless you pick one.").into()), language, RowPos::Last, main_tokens));
        self.settings_frame(layout, col)
    }

    fn render_settings_player(&mut self, layout: &Layout, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let main_tokens = f32::from(layout.main_width) / t.token_scale();
        let zoom = self.prefs.zoom as f32;
        let size = self.render_slider(SliderSpec::new("interface-size", zoom, ZOOM_MIN, ZOOM_MAX, |this, v, cx| this.set_zoom(v.round() as u32, cx)).step(ZOOM_STEP).look(SliderLook::Plain), cx);
        let position = segmented_settings(&t, "position", self.prefs.mode, &[(PlayerMode::Dock, tr("Right Dock"), None), (PlayerMode::Bar, tr("Bottom Bar"), None)], cx, |this, value, cx| this.set_mode(value, cx)).into_any_element();
        let blur = switch(&t, "lyric-blur", self.prefs.lyric_blur, true, cx, |this, cx| {
            this.prefs.lyric_blur = !this.prefs.lyric_blur;
            this.prefs.save();
            cx.notify();
        }).into_any_element();
        let col = div()
            .flex()
            .flex_col()
            .child(section_title(&t, tr("Player")))
            .child(settings_row(&t, tr("Interface size"), Some(trf("{zoom}%. Ctrl and plus or minus also changes it, and Ctrl+0 resets it.", &[("zoom", &self.prefs.zoom.to_string())]).into()), size, RowPos::First, main_tokens))
            .child(settings_row(&t, tr("Position"), Some(tr("On narrow windows the dock becomes the bottom bar until there's room.").into()), position, RowPos::Middle, main_tokens))
            .child(settings_row(&t, tr("Blur lyrics by distance"), None, blur, RowPos::Last, main_tokens));
        self.settings_frame(layout, col)
    }

    /// Startup and window: the tray and open at login, on Linux and Windows.
    fn render_settings_startup(&mut self, layout: &Layout, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        use crate::autostart::OpenAtLogin;
        let t = self.theme;
        let main_tokens = f32::from(layout.main_width) / t.token_scale();
        let close_options = Rc::new(vec![SelectOption::new("tray", tr("Keep playing in the tray")), SelectOption::new("quit", tr("Quit"))]);
        let close_value = if self.prefs.close_to_tray { "tray" } else { "quit" };
        let close = self.select_pill("close-window", close_value, close_options, tr("Choose"), Some(240.0), |this, value, cx| this.set_close_to_tray(value.as_ref() == "tray", cx), cx);
        let close_hint: Option<SharedString> = (self.prefs.close_to_tray && !self.tray_host_seen(cx)).then(|| tr("Your desktop isn't showing a tray right now, so closing the window quits Navia.").into());
        let login_options = Rc::new(vec![
            SelectOption::new(OpenAtLogin::Off.id(), tr("Off")),
            SelectOption::new(OpenAtLogin::On.id(), tr("On")),
            SelectOption::new(OpenAtLogin::Hidden.id(), tr("On, hidden in the tray")),
        ]);
        let mini = switch(&t, "mini-on-minimize", self.prefs.mini_on_minimize, true, cx, |this, cx| {
            this.prefs.mini_on_minimize = !this.prefs.mini_on_minimize;
            this.prefs.save();
            cx.notify();
        }).into_any_element();
        let login = self.select_pill("open-at-login", self.open_at_login.id(), login_options, tr("Choose"), Some(240.0), |this, value, cx| this.set_open_at_login(OpenAtLogin::from_id(value.as_ref()), cx), cx);
        let col = div()
            .flex()
            .flex_col()
            .child(section_title(&t, tr("Startup and window")))
            .child(settings_row(&t, tr("When I close the window"), close_hint, close, RowPos::First, main_tokens))
            .child(settings_row(&t, tr("Show the mini player when Navia is minimized"), None, mini, RowPos::Middle, main_tokens))
            .child(settings_row(&t, tr("Open at login"), None, login, RowPos::Last, main_tokens));
        self.settings_frame(layout, col)
    }

    /// Settings > About. Shows the version, plus update checks on Windows. Linux updates through its package manager.
    fn render_settings_about(&mut self, layout: &Layout, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        use crate::updates::{HAS_UPDATER, UpdateStatus};
        let t = self.theme;
        let main_tokens = f32::from(layout.main_width) / t.token_scale();
        let version = div().flex_none().text_size(t.text(TEXT_SM)).text_color(t.t2()).child(crate::updates::version_label()).into_any_element();
        let mut col = div().flex().flex_col().child(section_title(&t, tr("About")));
        // Navia's license, with the text on gnu.org.
        let license_button = settings_button(&t, "view-license", tr("View License"), true).stop(&t).on_click(|_, _, cx| cx.open_url(LICENSE_URL)).into_any_element();
        let license = settings_row(&t, tr("License"), Some(tr("Licensed under GPL-3.0-or-later.").into()), license_button, RowPos::Last, main_tokens);
        if !HAS_UPDATER {
            return self.settings_frame(layout, col.child(settings_row(&t, tr("Version"), None, version, RowPos::First, main_tokens)).child(license));
        }
        let status = self.update_status.clone();
        let label = match status {
            UpdateStatus::Checking => tr("Checking…"),
            UpdateStatus::Ready => tr("Restart to update"),
            _ => tr("Check for Updates"),
        };
        let checking = status == UpdateStatus::Checking;
        let button = settings_button(&t, "check-updates", label, !checking).when(!checking, |d| d.stop(&t).on_click(cx.listener(|this, _, _, cx| this.check_for_updates(cx))));
        let controls = div().flex().flex_row().items_center().flex_none().gap(t.s(10.0)).child(version).child(button).into_any_element();
        let auto = switch(&t, "check-updates-auto", self.prefs.check_updates, true, cx, |this, cx| {
            let on = !this.prefs.check_updates;
            this.set_check_updates(on, cx);
        })
        .into_any_element();
        col = col
            .child(settings_row(&t, tr("Version"), status.note().map(SharedString::from), controls, RowPos::First, main_tokens))
            .child(settings_row(&t, tr("Check for updates automatically"), None, auto, RowPos::Middle, main_tokens))
            .child(license);
        self.settings_frame(layout, col)
    }

    fn server_summary(&self) -> (SharedString, bool) {
        if self.sample_mode {
            return (tr("Sample data, for testing. The real app shows the server you connect to.").into(), false);
        }
        let Some(lib) = self.core.as_ref().map(|c| c.lib.clone()) else { return (tr("Not connected.").into(), false) };
        let host = lib.server_url().split("://").last().unwrap_or(lib.server_url()).trim_end_matches('/').to_string();
        let vars: &[(&str, &str)] = &[("host", &host), ("user", lib.username())];
        let text = if lib.from_env {
            trf("{host}, signed in as {user}. Set by NAVIDROME_URL, a developer override.", vars)
        } else if self.keyring_note {
            trf("{host}, signed in as {user}. No system keyring was found, so Navia can't remember your password.", vars)
        } else {
            trf("{host}, signed in as {user}.", vars)
        };
        (text.into(), !lib.from_env)
    }

    fn render_server_row(&mut self, main_tokens: f32, cx: &mut Context<Self>) -> gpui::Div {
        let t = self.theme;
        let (hint, switchable) = self.server_summary();
        let button = |id: &'static str, label: &'static str| settings_button(&t, id, label, switchable);
        let change = button("server-change", tr("Change Server")).when(switchable, |d| d.stop(&t).on_click(cx.listener(|this, _, _, cx| this.open_change_server(None, cx))));
        let sign_out = button("server-sign-out", tr("Sign Out")).when(switchable, |d| {
            d.stop(&t).on_click(cx.listener(|this, _, _, cx| {
                let host = this.core.as_ref().map_or(String::new(), |c| c.lib.server_url().split("://").last().unwrap_or("").trim_end_matches('/').to_string());
                this.confirm = Some(crate::view_connect::Confirm::SignOut { host: host.into() });
                cx.notify();
            }))
        });
        let actions = div().flex().flex_row().items_center().flex_none().gap(t.s(10.0)).child(change).child(sign_out).into_any_element();
        settings_row(&t, tr("Server"), Some(hint), actions, RowPos::First, main_tokens)
    }

    /// The Share Links row: how many links, or why sharing is off on the server.
    fn render_shares_row(&mut self, main_tokens: f32, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        if !self.sharing_on() {
            return row_shell(&t, RowPos::Last, main_tokens).child(row_text(&t, tr("Share Links"), Some(tr("Sharing is turned off on this server").into()), true)).into_any_element();
        }
        if self.shares.links.is_none() && self.shares.error.is_none() {
            self.refresh_shares();
        }
        let hint: SharedString = match self.shares.links.as_ref().map(Vec::len).filter(|n| *n > 0) {
            Some(n) => trn("{count} link", "{count} links", n).into(),
            None => tr("Links to songs, albums and playlists that anyone can open.").into(),
        };
        settings_link_row(&t, "shares-link", tr("Share Links"), hint, None, RowPos::Last, main_tokens)
            .stop(&t)
            .on_click(cx.listener(|this, _, _, cx| this.navigate(Route::Shares, cx)))
            .into_any_element()
    }

    fn render_settings_library(&mut self, layout: &Layout, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let main_tokens = f32::from(layout.main_width) / t.token_scale();
        let server = self.render_server_row(main_tokens, cx);
        let sync = switch(&t, "library-sync", self.prefs.library_sync, true, cx, |this, cx| {
            this.prefs.library_sync = !this.prefs.library_sync;
            this.prefs.save();
            if let Some(core) = this.core.as_ref() {
                core.lib.set_background_sync(this.prefs.library_sync);
            }
            cx.notify();
        }).into_any_element();
        let sync_hint: Option<SharedString> = (!self.prefs.library_sync).then(|| tr("Your library still updates when Navia starts.").into());
        // Synced downloads wait on a metered connection unless this is on.
        let metered_on = self.core.as_ref().is_some_and(|c| c.lib.downloads.download_on_metered());
        let metered = switch(&t, "download-metered", metered_on, self.core.is_some(), cx, move |this, cx| {
            if let Some(core) = this.core.as_ref() {
                let _ = core.lib.downloads.set_download_on_metered(!metered_on);
            }
            cx.notify();
        })
        .into_any_element();
        let col = div()
            .flex()
            .flex_col()
            .child(section_title(&t, tr("Library")))
            .child(server)
            .child(settings_row(&t, tr("Sync library in the background"), sync_hint, sync, RowPos::Middle, main_tokens))
            .child(self.render_quality_row_at(tr("Download quality"), tr(DOWNLOAD_QUALITY_HINT).into(), QualitySlot::Download, RowPos::Middle, main_tokens, cx))
            .child(settings_row(&t, tr("Download on metered connections"), Some(tr("Playlists and albums you keep downloaded wait while your system marks the connection as metered, like a phone hotspot. Turn this on to let them download there too. Downloading something yourself always runs.").into()), metered, RowPos::Middle, main_tokens))
            .child(self.render_shares_row(main_tokens, cx));
        self.settings_frame(layout, col)
    }

    fn render_settings_connections(&mut self, layout: &Layout, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let main_tokens = f32::from(layout.main_width) / t.token_scale();
        let private = switch(&t, "private-mode", self.private_mode, true, cx, |this, cx| {
            let on = !this.private_mode;
            this.set_private_mode(on, cx);
        }).into_any_element();
        // Share what I'm playing, opt-in and dimmed in Private mode.
        let share = switch(&t, "share-listening", self.prefs.share_listening, !self.private_mode, cx, |this, cx| {
            this.prefs.share_listening = !this.prefs.share_listening;
            this.prefs.save();
            this.apply_sharing(cx);
        })
        .into_any_element();
        let share_row = if self.private_mode {
            self.disabled_row(tr("Share what I'm playing"), tr("Paused while Private mode is on."), share, main_tokens)
        } else {
            settings_row(&t, tr("Share what I'm playing"), Some(tr(SHARE_HINT).into()), share, RowPos::Middle, main_tokens)
        };
        // Let listeners control playback. Only while sharing and not private.
        let control_row = (self.prefs.share_listening && !self.private_mode).then(|| {
            let control = switch(&t, "listeners-control", self.prefs.listeners_control, true, cx, |this, cx| {
                this.prefs.listeners_control = !this.prefs.listeners_control;
                this.prefs.save();
                this.apply_listen_marks();
                cx.notify();
            })
            .into_any_element();
            settings_row(&t, tr("Let listeners control playback"), Some(tr(CONTROL_HINT).into()), control, RowPos::Middle, main_tokens)
        });
        let discord_forced = crate::core_player::discord_override();
        let discord_on = discord_forced.map_or(self.prefs.discord_status, |_| self.discord_wanted());
        let discord = switch(&t, "discord-status", discord_on, discord_forced.is_none(), cx, |this, cx| {
            this.prefs.discord_status = !this.prefs.discord_status;
            this.prefs.save();
            this.apply_reporting();
            this.publish_integrations(true);
            cx.notify();
        }).into_any_element();
        let discord_hint: SharedString = match discord_forced {
            Some(why) => why.into(),
            None if self.private_mode => tr("Show what you're listening to on your profile. Off while Private mode is on.").into(),
            None => tr("Show what you're listening to on your profile.").into(),
        };
        let art_value = if self.prefs.discord_art == DiscordArt::Default { "default" } else { "public" };
        let art_options = Rc::new(vec![
            SelectOption::new("public", tr("Match public covers")),
            SelectOption::new("default", tr("Default image")),
        ]);
        let discord_art = self.select_pill("discord-art", art_value, art_options, tr("Choose"), Some(240.0), |this, value, cx| {
            this.prefs.discord_art = if value == "default" { DiscordArt::Default } else { DiscordArt::MatchPublic };
            this.prefs.save();
            this.apply_reporting();
            this.publish_integrations(true);
            cx.notify();
        }, cx);
        use navia_integrations::art::CoverSource;
        let covers_value = match self.prefs.discord_covers_from {
            CoverSource::Automatic => "auto",
            CoverSource::Server => "server",
            CoverSource::Search => "search",
        };
        let covers_options = Rc::new(vec![
            SelectOption::new("auto", tr("Automatic")),
            SelectOption::new("server", tr("My server")),
            SelectOption::new("search", tr("External sources")),
        ]);
        let covers_from = self.select_pill("discord-covers", covers_value, covers_options, tr("Choose"), Some(240.0), |this, value, cx| {
            this.prefs.discord_covers_from = match value.as_ref() {
                "server" => CoverSource::Server,
                "search" => CoverSource::Search,
                _ => CoverSource::Automatic,
            };
            this.prefs.save();
            this.publish_integrations(true);
            cx.notify();
        }, cx);
        let address_row = self.render_discord_address_row(main_tokens, cx);
        let scrobble_forced = crate::core_player::scrobble_override();
        let report_on = scrobble_forced.map_or(self.scrobble_setting(), |_| self.reporting_wanted());
        let scrobble = switch(&t, "scrobble", report_on, scrobble_forced.is_none(), cx, |this, cx| {
            this.toggle_scrobble();
            cx.notify();
        }).into_any_element();
        let scrobble_hint: SharedString = match scrobble_forced {
            Some(why) => why.into(),
            None if self.private_mode => tr("Sends plays to the server, which forwards them to whichever services it has set up. Off while Private mode is on.").into(),
            None => tr("Sends plays to the server, which forwards them to whichever services it has set up.").into(),
        };
        let (count_row, cap_row) = self.render_play_threshold(main_tokens, cx);
        let col = div()
            .flex()
            .flex_col()
            .child(section_title(&t, tr("Connections")))
            .child(settings_row(&t, tr("Private Mode"), Some(tr("Plays aren't counted or sent to scrobbling, others on the server can't see what you're playing, and Discord status is hidden.").into()), private, RowPos::First, main_tokens))
            .child(share_row)
            .children(control_row)
            .child(settings_row(&t, tr("Discord status"), Some(discord_hint), discord, RowPos::Middle, main_tokens))
            .child(settings_row(&t, tr("Discord album art"), None, discord_art, RowPos::Middle, main_tokens))
            .child(settings_row(&t, tr("Covers from"), Some(tr("If covers show up blank in Discord, choose External sources.").into()), covers_from, RowPos::Middle, main_tokens))
            .children(address_row)
            .child(settings_row(&t, tr("Scrobble plays"), Some(scrobble_hint), scrobble, RowPos::Middle, main_tokens))
            .child(count_row)
            .child(cap_row);
        self.settings_frame(layout, col)
    }

    /// "Count a play after" and "Or after 4 minutes", saved with the account in the core. Dimmed with the reason while
    /// scrobbling is off or Private mode is on. A developer override of scrobbling doesn't dim them.
    fn render_play_threshold(&mut self, main_tokens: f32, cx: &mut Context<Self>) -> (gpui::Div, gpui::Div) {
        let t = self.theme;
        let threshold = self.core.as_ref().map(|c| c.lib.reporter.play_threshold());
        let reason: Option<&'static str> = match threshold {
            None => Some(tr("Needs a connection to the server.")),
            Some(_) if self.private_mode => Some(tr("Paused while Private mode is on.")),
            Some(_) if !self.scrobble_setting() => Some(tr("Used while Scrobble plays is on.")),
            Some(_) => None,
        };
        let current = threshold.unwrap_or_default();
        let percent_label = |p: u8| trf("{percent}%", &[("percent", &p.to_string())]);
        let count_control = match reason {
            Some(_) => disabled_pill(&t, percent_label(current.percent)),
            None => {
                let options = Rc::new(PlayThreshold::PERCENT_CHOICES.iter().map(|&p| SelectOption::new(p.to_string(), percent_label(p))).collect::<Vec<_>>());
                self.select_pill("play-threshold", current.percent.to_string(), options, tr("Choose"), Some(240.0), |this, value, cx| {
                    let Ok(percent) = value.parse::<u8>() else { return };
                    this.set_play_threshold(|t| t.percent = percent);
                    cx.notify();
                }, cx)
            }
        };
        let cap = switch(&t, "play-threshold-cap", current.cap_at_four_minutes, reason.is_none(), cx, |this, cx| {
            this.set_play_threshold(|t| t.cap_at_four_minutes = !t.cap_at_four_minutes);
            cx.notify();
        }).into_any_element();
        let dim = reason.is_some();
        let count_hint = reason.unwrap_or(tr("Only the time you actually hear counts, and songs under 30 seconds never count."));
        let cap_hint = reason.unwrap_or(tr("Long songs also count once you've heard 4 minutes of them."));
        (
            row_shell(&t, RowPos::Middle, main_tokens).child(row_text(&t, tr("Count a play after"), Some(count_hint.into()), dim)).child(count_control),
            row_shell(&t, RowPos::Last, main_tokens).child(row_text(&t, tr("Or after 4 minutes"), Some(cap_hint.into()), dim)).child(cap),
        )
    }

    fn set_play_threshold(&mut self, change: impl FnOnce(&mut PlayThreshold)) {
        let Some(core) = self.core.as_ref() else { return };
        let mut next = core.lib.reporter.play_threshold();
        change(&mut next);
        if let Err(e) = core.lib.reporter.set_play_threshold(next) {
            eprintln!("navia: couldn't save when a play counts: {e}");
        }
    }
}

impl Shell {
    /// Settings > Spotify Friends: the friend list, adding one by Discord id, and the Lanyard address. Nothing contacts
    /// Lanyard until a friend is added.
    fn render_settings_friends(&mut self, layout: &Layout, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        use navia_core::lanyard::{self as ly, Reach};
        let t = self.theme;
        let main_tokens = f32::from(layout.main_width) / t.token_scale();
        let about = crate::view_album::text_button_on(&t, "lanyard-about", None, tr("About Lanyard"), true, t.t1().into())
            .stop(&t)
            .on_click(cx.listener(|_, _, _, cx| cx.open_url(ly::PROJECT_URL)))
            .into_any_element();
        let mut col = div().flex().flex_col().child(section_title(&t, tr("Spotify Friends"))).child(settings_row(&t, tr("Spotify friends"), Some(tr(FRIENDS_HINT).into()), about, RowPos::First, main_tokens));
        let saved = self.prefs.spotify_friends.clone();
        for (k, friend) in saved.into_iter().enumerate() {
            let now = self.listen.friends.iter().find(|f| f.friend.id == friend.id).cloned();
            let name = now.as_ref().map_or_else(|| friend.name.clone().unwrap_or_else(|| friend.id.clone()), |f| f.display_name());
            let status: String = match now.as_ref() {
                None => tr("Checking…").to_string(),
                Some(f) => match (f.reach, f.track.as_ref()) {
                    (Reach::Waiting, _) => tr("Checking…").to_string(),
                    (Reach::NotMonitored, _) => tr("Lanyard can’t see them yet. They need to join Lanyard’s Discord server.").to_string(),
                    (Reach::Unreachable, _) => tr("Can’t reach Lanyard right now.").to_string(),
                    (Reach::Readable, None) => tr("Not listening on Spotify right now.").to_string(),
                    (Reach::Readable, Some(track)) if f.song.is_some() => trf("Listening to “{song}” by {artist}.", &[("song", &track.song), ("artist", &track.artist_line())]),
                    (Reach::Readable, Some(track)) => trf("Listening to “{song}” by {artist}. Not in your library.", &[("song", &track.song), ("artist", &track.artist_line())]),
                },
            };
            let id = friend.id.clone();
            let remove = crate::view_album::text_button_on(&t, ("friend-remove", k), None, tr("Remove"), true, t.t1().into())
                .stop(&t)
                .on_click(cx.listener(move |this, _, _, cx| this.remove_friend(&id, cx)))
                .into_any_element();
            col = col.child(settings_row(&t, name, Some(status.into()), remove, RowPos::Middle, main_tokens));
        }
        // The id, checked as typed, and an optional name.
        let draft = self.listen.draft_id.clone();
        let parsed = ly::parse_friend_id(&draft);
        let taken = parsed.as_ref().is_some_and(|id| self.prefs.spotify_friends.iter().any(|f| &f.id == id));
        let add_hint: SharedString = if !draft.is_empty() && parsed.is_none() {
            tr("A Discord user ID is a number of 17 to 20 digits.").into()
        } else if taken {
            tr("This friend is already added.").into()
        } else {
            tr(FRIEND_ID_HINT).into()
        };
        let id_field = self.settings_field(TextTarget::FriendId, "friend-id", tr("Discord user ID"), 200.0, cx);
        let name_field = self.settings_field(TextTarget::FriendName, "friend-name", tr("Name (optional)"), 180.0, cx);
        let can_add = parsed.is_some() && !taken;
        let add = crate::view_album::text_button_on(&t, "friend-add", Some("plus-regular"), tr("Add"), can_add, t.t1().into())
            .stop(&t)
            .on_click(cx.listener(|this, _, _, cx| {
                this.add_friend(cx);
            }))
            .into_any_element();
        let controls = div().flex().flex_row().flex_wrap().items_center().justify_end().gap(t.s(8.0)).child(id_field).child(name_field).child(add).into_any_element();
        col = col.child(settings_row(&t, tr("Add a friend"), Some(add_hint), controls, RowPos::Middle, main_tokens));
        let address_ok = ly::parse_address(&self.prefs.lanyard_address).is_some();
        let address_hint: SharedString = if address_ok { tr("Lanyard’s public service, or the address of your own.").into() } else { tr("Enter the address as https://lanyard.example.org.").into() };
        let address = self.settings_field(TextTarget::LanyardAddress, "lanyard-address", ly::PUBLIC_ADDRESS, 240.0, cx);
        col = col.child(settings_row(&t, tr("Lanyard address"), Some(address_hint), address, RowPos::Last, main_tokens));
        self.settings_frame(layout, col)
    }

    /// A text field in a settings row, `width` tokens wide.
    fn settings_field(&mut self, target: TextTarget, key: &'static str, placeholder: &'static str, width: f32, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let value = self.text_value(&target);
        let line = self.field_line(&target, &value, placeholder, &format!("{key}-text"), t.text(TEXT_MD));
        let focused = self.focused_target() == Some(target.clone());
        crate::ui::dialog_field(&t, key, focused, None, line)
            .flex_none()
            .w(t.s(width))
            .on_click(cx.listener(move |this, _, _, cx| this.focus_field(target.clone(), cx)))
            .into_any_element()
    }
}
