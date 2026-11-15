//! The design tokens as a Rust theme. The Mac app's `Tokens.swift` uses the same names and values.
//!
//! Numbers are token units drawn at 87.5%, so `Theme::s` multiplies by 0.875 and by the user's
//! interface size. At 100%, `TEXT_MD` (16) is 14px.

use gpui::{Hsla, Pixels, Rgba, px, rgb, rgba};

use crate::color::{Srgb, apca_lc};

/// Screen pixels per token unit at 100% interface size.
pub const BASE_SCALE: f32 = 0.875;

// Colors that are the same in every theme.
pub const BG: u32 = 0x171718;
pub const PANEL: u32 = 0x1f1f21;
pub const PANEL_2: u32 = 0x2a2a2d;
/// What the expanded player shows before its backdrop is baked.
pub const EXPANDED_BG: u32 = 0x101014;
/// The danger color.
pub const DANGER: u32 = 0xe5484d;
/// The tint used when an album has none.
pub const TINT_FALLBACK: u32 = 0x34506b;
/// The shade the dock's gradient darkens toward at its foot. The player shades that follow are
/// the expanded player's diagonal gradient corners.
pub const DOCK_SHADE: u32 = 0x0a0e16;
pub const PLAYER_SHADE_START: u32 = 0x081018;
pub const PLAYER_SHADE_END: u32 = 0x060a14;
/// How much of the tint stays where the dock, mini player and expanded player darken toward a shade.
pub const PLAYER_SHADE_MIX: f32 = 0.72;
/// The inset 1px ring and faint top highlight of every glass surface.
pub const GLASS_EDGE: u32 = 0xffffff29;
pub const GLASS_EDGE_TOP: u32 = 0xffffff0f;
/// A filled button's faint light hairline, for accent, danger and selected segments. Glass
/// buttons already have `GLASS_EDGE`.
pub const BUTTON_EDGE: u32 = 0xffffff38;
/// The dark fade at the top of the dock, under its buttons.
pub const DOCK_HEAD_SHADE: u32 = 0x00000057;
/// A window button's fill over artwork, where a bare glyph can't be read on a bright cover.
pub const WINDOW_BUTTON_ON_ART: u32 = 0x00000038;
pub const WINDOW_BUTTON_ON_ART_HOVER: u32 = 0x00000059;
/// The EQ graph's dB rows, band columns and 0 dB line, and the rings around its handles.
pub const EQ_GRID: u32 = 0xffffff17;
pub const EQ_GRID_BAND: u32 = 0xffffff0a;
pub const EQ_GRID_ZERO: u32 = 0xffffff42;
pub const EQ_HANDLE_RING: u32 = 0x00000040;
pub const EQ_HANDLE_ACTIVE_RING: u32 = 0xffffff4d;
/// A status box's fill. The danger values below are a problem's fill alpha and how much
/// danger is mixed with white for its icon. See `danger_fill` and `danger_ink`.
pub const STATUS_FILL: u32 = 0xffffff0f;
pub const DANGER_FILL_ALPHA: f32 = 0.14;
/// The ring around an invalid field.
pub const INVALID_RING: u32 = 0xff7878b3;
/// The invalid ring's width, thin so a problem is marked quietly in place.
pub const INVALID_RING_W: f32 = 1.0;
pub const DANGER_INK_MIX: f32 = 0.7;
/// The dark app icon's gradient, top to bottom, behind the sign-in screen and installer splash.
pub const BRAND_TOP: u32 = 0x3a3a40;
pub const BRAND_BOTTOM: u32 = 0x0b0b0d;

/// A problem's fill.
pub fn danger_fill() -> Hsla {
    Hsla::from(rgb(DANGER)).alpha(DANGER_FILL_ALPHA)
}

/// A problem's icon color.
pub fn danger_ink() -> Hsla {
    Hsla::from(rgb(DANGER)).blend(gpui::white().opacity(1.0 - DANGER_INK_MIX))
}
/// The overlay scrollbar thumb's white opacity, at rest and when hovered or dragged.
pub const SCROLL_THUMB: f32 = 0.32;
pub const SCROLL_THUMB_HOVER: f32 = 0.5;
/// The explicit "E" mark's fill and ink.
pub const EXPLICIT_FILL: u32 = 0xffffff38;
pub const EXPLICIT_INK: u32 = 0x000000d1;

/// Page alpha over the blurred desktop when translucency is on. Secondary text still has 4.5:1
/// contrast over a pure white wallpaper.
pub const BG_WINDOW_ALPHA: f32 = 0.82;

pub mod size {
    pub const TEXT_XS: f32 = 14.0;
    pub const TEXT_SM: f32 = 15.0;
    pub const TEXT_MD: f32 = 16.0;
    pub const TEXT_LG: f32 = 18.0;
    pub const TEXT_SECTION: f32 = 25.0;
    pub const TEXT_FEATURE: f32 = 21.0;
    pub const TEXT_SUBTITLE: f32 = 23.0;
    pub const TEXT_TITLE: f32 = 42.0;

    pub const R_XS: f32 = 6.0;
    pub const R_SM: f32 = 8.0;
    pub const R_MD: f32 = 12.0;
    pub const R_ROW: f32 = 13.0;
    pub const R_LG: f32 = 22.0;
    /// The corner radius of every floating panel.
    pub const R_FLOAT: f32 = 16.0;
    /// A submenu's width, its maximum height before it scrolls, and its gap from the parent menu.
    pub const SUBMENU_W: f32 = 260.0;
    pub const SUBMENU_MAX_H: f32 = 440.0;
    pub const SUBMENU_GAP: f32 = 4.0;
    /// The anchored popover's width and maximum height, for the smart rules panel and the Songs
    /// filters, which set their own width. Content scrolls past the maximum.
    pub const POP_W: f32 = 380.0;
    pub const POP_MAX_H: f32 = 440.0;
    /// Song filters. The chip's height, padding and glyph, the gap between chips and below the top
    /// bar, the filter popover's width and list height, the number field's width, the gap above the
    /// table header, and an active chip's maximum value width.
    pub const CHIP_H: f32 = 32.0;
    pub const CHIP_PAD: f32 = 13.0;
    pub const CHIP_GLYPH: f32 = 12.0;
    pub const CHIP_GAP: f32 = 6.0;
    pub const FILTER_ROW_GAP: f32 = 16.0;
    pub const CHIP_VALUE_MAX_W: f32 = 180.0;
    pub const FILTER_POP_W: f32 = 300.0;
    pub const FILTER_LIST_MAX_H: f32 = 300.0;
    pub const NUMBER_FIELD_W: f32 = 76.0;
    /// The smart playlist editor. Rule field and condition pills, the gap between rule rows, the
    /// indent per group level, the sort rows' pill and label column, and the limit field.
    pub const SMART_FIELD_W: f32 = 220.0;
    pub const SMART_OP_W: f32 = 175.0;
    pub const SMART_ROW_GAP: f32 = 8.0;
    pub const SMART_GROUP_INDENT: f32 = 14.0;
    pub const SMART_SORT_W: f32 = 220.0;
    pub const SMART_SORT_LABEL_W: f32 = 64.0;
    pub const SMART_LIMIT_W: f32 = 140.0;
    /// How far a toast floats above the bottom of the content column.
    pub const TOAST_GAP: f32 = 32.0;
    /// Listening Now's rows. The cover beside each person, and the live mark's three accent bars.
    pub const PERSON_ART: f32 = 30.0;
    pub const LIVE_MARK_H: f32 = 10.0;
    pub const LIVE_BAR_W: f32 = 2.0;
    pub const LIVE_BAR_GAP: f32 = 1.5;
    /// A Spotify friend's row. The friend mark on the avatar's corner, its glyph, and the initials
    /// shown without an avatar.
    pub const FRIEND_MARK: f32 = 13.0;
    pub const FRIEND_MARK_GLYPH: f32 = 8.0;
    pub const FRIEND_INITIALS: f32 = 11.0;
    /// A toast's height and side padding. One with an action keeps the height, with its small
    /// button inset evenly at the right end.
    pub const TOAST_H: f32 = 42.0;
    pub const TOAST_PAD: f32 = 18.0;
    /// How long a toast stays. One with an action stays longer, and hovering keeps it.
    pub const TOAST_MS: u64 = 3000;
    pub const TOAST_ACTION_MS: u64 = 5000;
    /// The space above each of the queue's section headings.
    pub const QUEUE_SECTION_GAP: f32 = 14.0;
    /// Placeholder rows under the Autoplay heading while its songs are being found.
    pub const QUEUE_FINDING_ROWS: usize = 3;
    /// A placeholder row's two text lines as quiet blocks. Their height, the gap between them and
    /// their widths as fractions of the row.
    pub const ROW_PLACEHOLDER_LINE: f32 = 9.0;
    pub const ROW_PLACEHOLDER_GAP: f32 = 8.0;
    pub const ROW_PLACEHOLDER_TITLE_W: f32 = 0.55;
    pub const ROW_PLACEHOLDER_ARTIST_W: f32 = 0.35;
    /// The glyph size and gap of the queue headings' icon buttons, Clear and Shuffle.
    pub const QUEUE_HEAD_ICON: f32 = 18.0;
    pub const QUEUE_HEAD_GAP: f32 = 8.0;
    /// The square X button that removes a queue row.
    pub const QUEUE_REMOVE_W: f32 = 28.0;
    /// Songs dragged out of their list. The card that follows the pointer at a
    /// `DRAG_OUT_OFFSET_X/Y` offset, the dimming of rows left behind, the drop target's ring, and how
    /// far a card's drop highlight spreads past its edges.
    pub const DRAG_OUT_CARD_W: f32 = 280.0;
    pub const DRAG_OUT_OFFSET_X: f32 = 14.0;
    pub const DRAG_OUT_OFFSET_Y: f32 = 12.0;
    pub const DRAG_OUT_DIM: f32 = 0.4;
    pub const DROP_RING: f32 = 2.0;
    pub const DROP_PAD_CARD: f32 = 8.0;
    /// The dialog's width, radius, padding, gaps and button padding. Buttons are `CONTROL_H`
    /// pills. `FIELD_PAD` is a text field's side padding.
    pub const DIALOG_W: f32 = 420.0;
    pub const DIALOG_PAD: f32 = 22.0;
    pub const DIALOG_GAP: f32 = 14.0;
    pub const DIALOG_ACTIONS_GAP: f32 = 10.0;
    pub const DIALOG_ACTIONS_TOP: f32 = 4.0;
    pub const DIALOG_BUTTON_PAD: f32 = 18.0;
    /// A menu's section heading, like the Output menu's AirPlay, under the 36-token rows.
    pub const MENU_HEADING_H: f32 = 30.0;
    /// A code entry's digit box size, the gap between boxes and the digit's text size.
    pub const PIN_BOX_W: f32 = 54.0;
    pub const PIN_BOX_H: f32 = 62.0;
    pub const PIN_GAP: f32 = 10.0;
    pub const PIN_TEXT: f32 = TEXT_SECTION;
    /// The padding around a command to copy, which uses `assets::font_mono`.
    pub const CODE_PAD: f32 = 12.0;
    /// The AirPlay output badge. The gap from glyph to name, the name's maximum width beside the
    /// bar's title, and the bar width below which only the glyph shows.
    pub const OUTPUT_BADGE_GAP: f32 = 6.0;
    pub const OUTPUT_BADGE_MAX_W: f32 = 200.0;
    pub const OUTPUT_BADGE_NARROW: f32 = 960.0;
    /// One turn of the busy glyph, in milliseconds.
    pub const SPIN_PERIOD_MS: u64 = 900;
    /// The small glass text button's height and padding.
    pub const BUTTON_SMALL_H: f32 = 30.0;
    pub const BUTTON_SMALL_PAD: f32 = 12.0;
    /// The gap between a button's glyph and its label.
    pub const BUTTON_GAP: f32 = 8.0;
    pub const FIELD_PAD: f32 = 15.0;
    /// A labeled text field. Label to field, field to message, and an inner action's inset.
    pub const FIELD_LABEL_GAP: f32 = 7.0;
    pub const FIELD_MESSAGE_GAP: f32 = 7.0;
    pub const FIELD_INSET: f32 = 3.0;
    /// A status box's padding and the gap after its icon.
    pub const STATUS_PAD_Y: f32 = 11.0;
    pub const STATUS_PAD_X: f32 = 14.0;
    pub const STATUS_GAP: f32 = 12.0;
    /// A prompt card's padding, and the gap between its title, fact and answers.
    pub const PROMPT_CARD_PAD: f32 = 16.0;
    pub const PROMPT_CARD_GAP: f32 = 10.0;
    /// The lockup's wordmark size and gap from the mark, as fractions of the mark's size. Art is
    /// `assets/icons/nv-brand-*.svg`, from tools/brand_icons.py.
    pub const BRAND_WORD_RATIO: f32 = 1.1;
    pub const BRAND_GAP_RATIO: f32 = 0.3;
    /// The sign-in screen. The form's width, the gaps between fields and under the heading, the
    /// card's padding and the lockup's mark size.
    pub const CONNECT_W: f32 = 420.0;
    pub const CONNECT_GAP: f32 = 16.0;
    pub const CONNECT_HEADING_GAP: f32 = 8.0;
    pub const CONNECT_CARD_PAD: f32 = 28.0;
    pub const CONNECT_MARK: f32 = 55.0;
    /// The inset of window buttons in a corner of a screen with no top bar, like sign-in.
    pub const WINDOW_CORNER_GAP: f32 = 12.0;
    /// A disabled text button's opacity, or the alpha of a disabled primary or danger fill.
    pub const DISABLED_OPACITY: f32 = 0.4;
    /// The sheet dialog's width and maximum height, and its info rows' art, label column, padding
    /// and section gap.
    pub const SHEET_W: f32 = 560.0;
    pub const SHEET_MAX_H: f32 = 780.0;
    pub const INFO_ART: f32 = 88.0;
    pub const INFO_LABEL_W: f32 = 148.0;
    pub const INFO_ROW_PAD_Y: f32 = 9.0;
    pub const INFO_ROW_PAD_X: f32 = 14.0;
    pub const INFO_SECTION_GAP: f32 = 18.0;
    /// The cover sheet. The picture's height, how far text rises over its fade, its focus point and
    /// fade start as fractions of its height, and the gaps between and within blocks.
    pub const SHEET_COVER_H: f32 = 420.0;
    pub const SHEET_COVER_OVERLAP: f32 = 120.0;
    pub const SHEET_COVER_FOCUS_Y: f32 = 0.22;
    pub const SHEET_COVER_FADE_FROM: f32 = 0.45;
    pub const SHEET_SECTION_GAP: f32 = 22.0;
    pub const SHEET_LABEL_GAP: f32 = 8.0;
    /// Download marks. The mark on sidebar rows, cards and lists, the glyph in buttons and rows, and
    /// the gap before a mark.
    pub const DOWNLOAD_MARK: f32 = 14.0;
    pub const DOWNLOAD_GLYPH: f32 = 20.0;
    pub const DOWNLOAD_MARK_GAP: f32 = 6.0;

    /// The page's side margin, the same at every window width.
    pub const GUTTER: f32 = 45.0;
    /// The split view for Albums and Artists. The list's width and gap before the divider, the row's
    /// height, padding, picture, text gap, highlight inset and separator start, and the All row's
    /// glyph. The page beside it uses `SPLIT_GUTTER` and `SPLIT_END`. Below `SPLIT_MIN_W` it's a grid.
    pub const SPLIT_LIST_W: f32 = 315.0;
    pub const SPLIT_LIST_PAD: f32 = 20.0;
    pub const SPLIT_ROW_H: f32 = 78.0;
    pub const SPLIT_ROW_PAD: f32 = 5.0;
    pub const SPLIT_ART: f32 = 45.0;
    pub const SPLIT_ROW_GAP: f32 = 13.0;
    pub const SPLIT_ROW_INSET: f32 = 6.0;
    pub const SPLIT_LINE_FROM: f32 = 60.0;
    pub const SPLIT_GLYPH: f32 = 26.0;
    pub const SPLIT_GUTTER: f32 = 20.0;
    pub const SPLIT_END: f32 = 25.0;
    pub const SPLIT_MIN_W: f32 = 760.0;
    pub const CONTENT_TOP: f32 = 32.0;
    pub const SIDEBAR_W: f32 = 245.0;
    /// The range the sidebar can be dragged to, and the grab strip on its right edge, the same as
    /// `DOCK_GRAB_W`.
    pub const SIDEBAR_W_MIN: f32 = 200.0;
    pub const SIDEBAR_W_MAX: f32 = 360.0;
    pub const SIDEBAR_GRAB_W: f32 = DOCK_GRAB_W;
    pub const DOCK_W: f32 = 437.0;
    /// The range the user can drag the dock to.
    pub const DOCK_MIN_W: f32 = 340.0;
    pub const DOCK_MAX_W: f32 = 640.0;
    /// The grab strip on the dock's left edge, about 12px wide at 100%.
    pub const DOCK_GRAB_W: f32 = 12.0 / 0.875;
    /// The sidebar's backdrop blur, and its soft top edge.
    pub const SIDEBAR_BLUR: f32 = 38.0;
    pub const SIDEBAR_SATURATION: f32 = 1.6;
    pub const GLASS_BLUR: f32 = 24.0;
    pub const GLASS_SATURATION: f32 = 1.5;
    /// The backdrop blur of menus, popovers, dialogs and toasts.
    pub const FLOAT_BLUR: f32 = 30.0;
    pub const FLOAT_SATURATION: f32 = 1.6;
    /// The bottom bar's backdrop blur, stronger than a popover's so the bar isn't too transparent.
    pub const BAR_BLUR: f32 = 44.0;
    pub const BAR_SATURATION: f32 = 1.6;
    /// The expanded player's text scrim. Wide, soft and light so it never reads as a box.
    pub const GLOW_SCRIM_REACH: f32 = 96.0;
    pub const GLOW_SCRIM_BLUR: f32 = 64.0;
    pub const GLOW_SCRIM_MAX: f32 = 0.32;
    /// A synced lyric line's opacity by distance from the current line. Near, minus a step per line,
    /// never below the minimum.
    pub const LYRIC_DIM_NEAR: f32 = 0.46;
    pub const LYRIC_DIM_STEP: f32 = 0.07;
    pub const LYRIC_DIM_MIN: f32 = 0.14;
    /// White's opacity on the unsung part of the current lyric line.
    pub const LYRIC_UNSUNG: f32 = 0.58;
    /// A hovered lyric line's opacity.
    pub const LYRIC_HOVER: f32 = 0.9;
    /// A lyric line's blur by distance from the current line, in CSS pixels.
    pub const LYRIC_BLUR_NEAR: f32 = 1.2;
    pub const LYRIC_BLUR_STEP: f32 = 0.9;
    pub const LYRIC_BLUR_MAX: f32 = 5.5;
    /// The current line's soft white glow, its blur in CSS pixels and its strength.
    pub const LYRIC_GLOW_BLUR: f32 = 16.0;
    pub const LYRIC_GLOW_ALPHA: f32 = 0.28;
    /// The fraction of the space above the current lyric line that stays clear before the top fade
    /// starts, so lines leave before reaching the player's head.
    pub const LYRIC_TOP_CLEAR: f32 = 0.5;
    /// The opacity of unsynced lyrics shown as plain text.
    pub const LYRIC_PLAIN: f32 = 0.92;
    /// The bar glow's strength at its brightest.
    pub const BAR_GLOW_ALPHA: f32 = 0.24;
    /// The mini player. Its compact and large sizes, corners, padding and gaps, the compact one's
    /// buttons and window controls, how solid its tinted glass is, the large one's scrim, and how far
    /// from a screen corner it first opens.
    pub const MINI_W: f32 = 366.0;
    pub const MINI_H: f32 = 100.0;
    pub const MINI_LARGE: f32 = 366.0; // Same as MINI_W, so the large one is square.
    pub const MINI_R: f32 = R_FLOAT;
    pub const MINI_PAD: f32 = 12.0;
    pub const MINI_GAP: f32 = 12.0;
    pub const MINI_BUTTON: f32 = 34.0;
    pub const MINI_CHROME_BUTTON: f32 = 28.0;
    pub const MINI_GLASS: f32 = 0.86;
    pub const MINI_SCRIM_START: f32 = 0.34;
    pub const MINI_SCRIM_STRENGTH: f32 = 0.92;
    pub const MINI_CORNER_MARGIN: f32 = 16.0;
    /// The one-time hint's width, at least the card's, and its gap from the card, on desktops
    /// without layer-shell.
    pub const MINI_HINT_W: f32 = 380.0;
    /// Resizing, from the compact size up to `MINI_MAX` either way. At least `MINI_STACK_H` tall with
    /// height at least `MINI_STACK_RATIO` of width it uses the stacked layout. Otherwise it's compact
    /// with a square cover. `MINI_GRIP` is the resize band, doubled at the corners.
    pub const MINI_MAX: f32 = 640.0;
    pub const MINI_STACK_H: f32 = 220.0;
    pub const MINI_STACK_RATIO: f32 = 0.6;
    pub const MINI_GRIP: f32 = 8.0;
    pub const MINI_HINT_GAP: f32 = 10.0;
    /// The mini player's hairline seek bar, its hover thumb and its time chip's edge limit. A sync's
    /// progress bar height, and its width under the empty state's text.
    pub const SYNC_BAR_H: f32 = 3.0;
    pub const SYNC_METER_W: f32 = 280.0;
    pub const SCRUB_THIN: f32 = 3.0;
    pub const SCRUB_THIN_KNOB: f32 = 10.0;
    /// The thumb of the other seek bars and the volume slider, and its starting scale as it fades in.
    pub const SCRUB_KNOB: f32 = 12.0;
    pub const SCRUB_KNOB_REST_SCALE: f32 = 0.6;
    pub const SCRUB_CHIP_EDGE: f32 = 46.0;
    /// A pressed small control's opacity, play and skip's scale when hovered and pressed, and an
    /// icon button's opacity at rest.
    pub const PRESS_DIM: f32 = 0.7;
    pub const HOVER_GROW: f32 = 1.08;
    pub const PRESS_SHRINK: f32 = 0.94;
    pub const ICON_REST: f32 = 0.88;
    /// How much smaller the switch is inside a menu row.
    pub const SWITCH_COMPACT: f32 = 0.86;
    /// Shuffle and repeat's opacity while off, and the player switcher's unselected segments.
    pub const MODE_REST: f32 = 0.62;
    /// A disabled mode button, like Shuffle during a station.
    pub const MODE_DISABLED: f32 = 0.35;
    pub const SWITCHER_REST: f32 = 0.85;
    /// The player switcher's height, a 40-token segment inside 4 tokens of padding.
    pub const SWITCHER_H: f32 = 48.0;
    /// Blurs for soft copies of artwork. The dock's reflection, Discover's hero wash, the page glow
    /// and the expanded backdrop. The art bakes blur small copies, so their sigmas approximate these.
    pub const REFLECT_BLUR: f32 = 32.0;
    pub const REFLECT_SATURATION: f32 = 1.2;
    /// The dock reflection's opacity.
    pub const REFLECT_OPACITY: f32 = 0.52;
    /// The artist header. The photo's maximum aspect, its gap beside the name, the name's gap to its
    /// controls, and the gaps of the Top Songs and Latest Release columns side by side or stacked.
    pub const ARTIST_ART_MAX_RATIO: f32 = 1.6;
    pub const ARTIST_HEAD_GAP: f32 = 32.0;
    pub const ARTIST_HEAD_TITLE_GAP: f32 = 16.0;
    pub const ARTIST_COLUMNS_GAP: f32 = 44.0;
    pub const ARTIST_COLUMNS_STACK_GAP: f32 = 24.0;
    pub const PAGE_GLOW_SATURATION: f32 = 1.2;
    /// The strength of Discover's hero wash, the page glow and the expanded player's backdrop glow.
    pub const HERO_GLOW_OPACITY: f32 = 0.6;
    pub const PAGE_GLOW_OPACITY: f32 = 0.72;
    pub const PLAYER_GLOW_OPACITY: f32 = 0.55;
    /// The brightness multiplier applied in the page and player glows.
    pub const PAGE_GLOW_BRIGHTNESS: f32 = 0.72;
    pub const PLAYER_GLOW_BRIGHTNESS: f32 = 0.82;
    pub const PLAYER_GLOW_SATURATION: f32 = 1.3;
    /// The expanded backdrop glow's deviation, in CSS pixels.
    pub const PLAYER_GLOW_BLUR: f32 = 110.0;
    pub const EDGE_BLUR: f32 = 24.0;
    pub const EDGE_SATURATION: f32 = 1.4;
    /// The lower part of the top edge that fades out.
    pub const EDGE_FADE: f32 = 0.45;
    /// A slight page-color dimming so controls read over the blur.
    pub const EDGE_TINT: f32 = 0.62;
    /// The margin the sidebar, dock and bottom bar keep from the window's edges.
    pub const WINDOW_INSET: f32 = 10.0;
    /// The window's own corner radius, 10px at 100% on every desktop.
    pub const WINDOW_RADIUS: f32 = 10.0 / 0.875;
    /// The window's outline and shadow, in real pixels outside the interface scale. Only for Linux
    /// client-side decorations, since Windows and macOS draw their own.
    pub const FRAME_OUTLINE_WIDTH: f32 = 1.0;
    pub const FRAME_OUTLINE_ALPHA: f32 = 0.11;
    pub const FRAME_SHADOW_BLUR: f32 = 20.0;
    pub const FRAME_SHADOW_ALPHA: f32 = 0.42;
    pub const FRAME_SHADOW_ALPHA_INACTIVE: f32 = 0.2;
    /// Room around the window for the shadow. GPUI's blur reaches about 1.8 times its radius, so a
    /// smaller margin would cut it off. Only `FRAME_GRIP_OUTSIDE` of it takes the pointer.
    pub const FRAME_SHADOW_MARGIN: f32 = 36.0;
    /// How far the resize grips reach out over the shadow and into the window. Thin like native
    /// grips, which run about 4 to 8 px.
    pub const FRAME_GRIP_OUTSIDE: f32 = 8.0;
    pub const FRAME_GRIP_INSIDE: f32 = 4.0;
    /// How far a corner grip extends past the edge grips, so corners are easier to catch.
    pub const FRAME_GRIP_CORNER: f32 = 10.0;
    pub const BAR_W: f32 = 1010.0;
    pub const BAR_H: f32 = 62.0;
    /// The player chip over the bar, centered above the play controls. Its maximum width and gap.
    pub const CHIP_ABOVE_MAX_W: f32 = 360.0;
    pub const CHIP_ABOVE_GAP: f32 = 10.0;
    pub const TOPBAR_H: f32 = 76.0;
    pub const PILL_H: f32 = 56.0;
    pub const CONTROL_H: f32 = 40.0;
    pub const ICON_W: f32 = 38.0;
    pub const ACTION_W: f32 = 41.0;
    pub const ROW_H: f32 = 78.0;
    /// A list page like Downloads. Its maximum width, the gap between its parts and its rows' padding.
    pub const LIST_PAGE_W: f32 = 980.0;
    pub const LIST_PAGE_GAP: f32 = 30.0;
    pub const LIST_ROW_PAD_X: f32 = 10.0;
    /// A list page's song or collection row height.
    pub const LIST_ROW_H: f32 = 68.0;
    /// Share links. The cover's gap to its title, the end-date and visits columns, and the button gap.
    pub const SHARE_SUBJECT_GAP: f32 = 12.0;
    pub const SHARE_ENDS_W: f32 = 170.0;
    pub const SHARE_VISITS_W: f32 = 170.0;
    pub const SHARE_ACTIONS_GAP: f32 = 2.0;
    pub const ROW_COMPACT_H: f32 = 44.0;
    pub const THUMB: f32 = 42.0;
    /// Card and grid sizes, and their narrow-window values (`NARROW_*`) with title sizes.
    pub const CARD_W: f32 = 200.0;
    pub const CARD_GAP: f32 = 32.0;
    pub const TILE_GAP: f32 = 20.0;
    pub const GRID_CARD: f32 = 224.0;
    pub const SECTION_GAP: f32 = 42.0;
    /// A section heading's gap above its rows or cards, and to a button at its right.
    pub const SHELF_HEAD_GAP: f32 = 16.0;
    /// How far past the window the page list builds blocks, so a later section is requested only
    /// once it's this close to view.
    pub const NEAR_VIEW_MARGIN: f32 = 420.0;
    /// The gap below a playlist's action pills before the sorted-order note. The table follows the
    /// note directly, so the column labels sit about as far below it.
    pub const ORDER_NOTE_GAP: f32 = 28.0;
    pub const HERO_ART: f32 = 250.0;
    pub const NARROW_CARD_W: f32 = 148.0;
    pub const NARROW_CARD_GAP: f32 = 14.0;
    pub const NARROW_TILE_GAP: f32 = 14.0;
    pub const NARROW_GRID_CARD: f32 = 150.0;
    pub const NARROW_SECTION_GAP: f32 = 30.0;
    pub const NARROW_TEXT_SECTION: f32 = 20.0;
    pub const NARROW_TEXT_TITLE: f32 = 30.0;
    pub const NARROW_HERO_ART: f32 = 180.0;
    /// Compact two-line rows, such as album tracks and genre rows.
    pub const MINI_ROW_H: f32 = 65.0;
    /// Genre tiles' width over height, and the genre rows' minimum width.
    pub const GENRE_TILE_RATIO: f32 = 4.0 / 3.0;
    pub const GENRE_CHIP_W: f32 = 250.0;
    /// The name's inset from a genre tile's top and left, as fractions of the tile's width.
    pub const GENRE_TILE_PAD_Y: f32 = 0.07;
    pub const GENRE_TILE_PAD_X: f32 = 0.08;
    pub const NARROW_GENRE_CHIP_W: f32 = 200.0;
    /// A genre row's side padding and the gap between its cover and text.
    pub const GENRE_ROW_PAD: f32 = 8.0;
    pub const GENRE_ROW_GAP: f32 = 13.0;
    /// The fan of covers in a genre tile's corner. A cover's width as a fraction of the tile's, the
    /// front cover's overhang past the right and bottom edges, each card's shift and rotation front to
    /// back at rest and on hover, and the single-cover case.
    pub const GENRE_FAN_SIZE: f32 = 0.46;
    pub const GENRE_FAN_RIGHT: f32 = -0.05;
    pub const GENRE_FAN_BOTTOM: f32 = -0.17;
    pub const GENRE_FAN: [FanCard; 3] = [FanCard { x: 0.0, y: 0.0, deg: 10.0 }, FanCard { x: -0.30, y: 0.0, deg: -2.0 }, FanCard { x: -0.56, y: 0.0, deg: -14.0 }];
    pub const GENRE_FAN_HOVER: [FanCard; 3] = [FanCard { x: 0.0, y: -0.03, deg: 13.0 }, FanCard { x: -0.34, y: -0.02, deg: -3.0 }, FanCard { x: -0.64, y: 0.0, deg: -18.0 }];
    pub const GENRE_FAN_SINGLE: FanCard = FanCard { x: 0.0, y: 0.0, deg: 6.0 };
    /// A tinted tile's gradient angle and how much the far end darkens.
    pub const TILE_FILL_ANGLE: f32 = 150.0;
    pub const TILE_FILL_SHADE: f32 = 0.7;
    /// The maximum lightness a tint background may have under light text.
    pub const TINT_TEXT_SAFE_L: f32 = 0.47;
    /// How far a card's art or a tile rises on hover.
    pub const CARD_LIFT: f32 = 3.0;
    /// The quiet "Mix" chip on a cover the app made. Its inset from the corner and its padding.
    pub const COVER_LABEL_INSET: f32 = 8.0;
    pub const COVER_LABEL_PAD_Y: f32 = 2.0;
    pub const COVER_LABEL_PAD_X: f32 = 8.0;
    /// The gap from a card's art or tile to the text under it.
    pub const CARD_TEXT_GAP: f32 = 10.0;
    /// Artist Radio tiles, as fractions of the tile's width. The name's padding, the photos' center
    /// height, the seed and neighbor photo sizes, the separating ring, the neighbor offset at rest and
    /// on hover, and the name's size, smaller when it doesn't fit.
    pub const STATION_PAD: f32 = 0.06;
    pub const STATION_FACES_Y: f32 = 0.46;
    pub const STATION_FACE: f32 = 0.50;
    pub const STATION_SIDE_FACE: f32 = 0.36;
    pub const STATION_RING: f32 = 0.015;
    pub const STATION_SIDE_SHIFT: f32 = 0.36;
    pub const STATION_SIDE_SHIFT_HOVER: f32 = 0.39;
    pub const STATION_NAME: f32 = 0.13;
    pub const STATION_NAME_LONG: f32 = 0.095;
    /// The large art shadow as (color, y offset, blur, spread).
    pub const SHADOW_ART: (u32, f32, f32, f32) = (0x000000b3, 20.0, 48.0, -20.0);
    /// The floating panel shadow as (color, y offset, blur, spread).
    pub const SHADOW_FLOAT: (u32, f32, f32, f32) = (0x000000b3, 18.0, 50.0, -14.0);

    /// One card of a fan.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct FanCard {
        pub x: f32,
        pub y: f32,
        pub deg: f32,
    }

    pub const DOCK_MIN_WINDOW: f32 = 1180.0;
    pub const SIDEBAR_MIN_WINDOW: f32 = 860.0;
    /// The narrow-window width, where the sidebar becomes a drawer.
    pub const NARROW_WINDOW: f32 = SIDEBAR_MIN_WINDOW;
    /// The narrowest page beside the dock, at the narrowest window that can dock with the sidebar
    /// open and the dock at its default width.
    pub const DOCK_MIN_CONTENT: f32 = DOCK_MIN_WINDOW - SIDEBAR_W - WINDOW_INSET * 2.0 - DOCK_W;
}

/// Animation lengths in seconds and the Reduce Motion switch. `progress` and `follow` finish at
/// once while motion is reduced, and GPUI's `with_animation` follows `App::reduce_motion`. Things
/// that show information over time, like progress bars, don't go through here.
pub mod motion {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    /// Hover timing. Changes take `HOVER_IN` going on and `HOVER_OUT` going off, and a press
    /// `HOVER_PRESS`, all on the `HOVER_EASE` cubic-bezier curve.
    pub const HOVER_IN: f32 = 0.10;
    pub const HOVER_OUT: f32 = 0.24;
    pub const HOVER_PRESS: f32 = 0.10;
    pub const HOVER_EASE: [f32; 4] = [0.2, 0.7, 0.2, 1.0];
    /// How long revealed controls stay after the pointer leaves, before they fade over `HOVER_OUT`.
    pub const REVEAL_GRACE: f32 = 0.3;
    /// Rows sliding apart and closing up during a drag.
    pub const DRAG_SLIDE: f32 = 0.17;
    /// A page's tint fading to another.
    pub const TINT_FADE: f32 = 0.32;
    /// Art fading in over its placeholder.
    pub const ART_FADE: f32 = 0.18;
    /// An overlay scrollbar fading out after scrolling stops.
    pub const SCROLLBAR_FADE: f32 = 0.2;
    /// Time constants for the lyrics gliding to the current line and lines easing to their dim and blur.
    pub const LYRIC_GLIDE: f32 = 0.17;
    pub const LYRIC_DIM: f32 = 0.11;
    /// Sync progress. Without a total, a segment `SYNC_SWEEP_SPAN` of the bar wide sweeps across every
    /// `SYNC_SWEEP` seconds. A later sync shows only once it has run `SYNC_SHOW_AFTER`.
    pub const SYNC_SWEEP: f32 = 1.4;
    pub const SYNC_SWEEP_SPAN: f32 = 0.3;
    pub const SYNC_SHOW_AFTER: f32 = 10.0;

    static REDUCED: AtomicBool = AtomicBool::new(false);

    /// Whether the system asks for reduced motion.
    pub fn reduced() -> bool {
        REDUCED.load(Ordering::Relaxed)
    }

    /// Sets reduced motion from the settings portal and redraws the windows when it changes.
    pub fn set_reduced(cx: &mut gpui::App, on: bool) {
        if REDUCED.swap(on, Ordering::Relaxed) != on {
            eprintln!("navia: reduce motion {}", if on { "on" } else { "off" });
        }
        cx.set_reduce_motion(on);
    }

    /// How far an animation `secs` long has come after `elapsed` seconds, from 0 to 1.
    pub fn progress(elapsed: f32, secs: f32) -> f32 {
        if reduced() || secs <= 0.0 { 1.0 } else { (elapsed / secs).clamp(0.0, 1.0) }
    }

    /// The fraction of the remaining distance a value following its goal with time constant `secs`
    /// covers in `dt`.
    pub fn follow(dt: f32, secs: f32) -> f32 {
        if reduced() || secs <= 0.0 { 1.0 } else { 1.0 - (-dt / secs).exp() }
    }

    /// `HOVER_EASE` at linear progress `p`, solving the cubic-bezier for x with Newton's method.
    pub fn hover_ease(p: f32) -> f32 {
        cubic_bezier(HOVER_EASE, p)
    }

    /// A CSS `cubic-bezier(x1, y1, x2, y2)` timing function at `p`.
    pub fn cubic_bezier([x1, y1, x2, y2]: [f32; 4], p: f32) -> f32 {
        let p = p.clamp(0.0, 1.0);
        if p <= 0.0 || p >= 1.0 {
            return p;
        }
        let curve = |a: f32, b: f32, s: f32| 3.0 * a * s * (1.0 - s) * (1.0 - s) + 3.0 * b * s * s * (1.0 - s) + s * s * s;
        let slope = |a: f32, b: f32, s: f32| 3.0 * a * (1.0 - s) * (1.0 - s) + 6.0 * (b - a) * s * (1.0 - s) + 3.0 * (1.0 - b) * s * s;
        let mut s = p;
        for _ in 0..8 {
            let dx = slope(x1, x2, s);
            if dx.abs() < 1e-6 {
                break;
            }
            s = (s - (curve(x1, x2, s) - p) / dx).clamp(0.0, 1.0);
        }
        curve(y1, y2, s)
    }

    /// A length for GPUI's `Animation`.
    pub fn duration(secs: f32) -> Duration {
        Duration::from_secs_f32(secs)
    }

    /// Play and skip's hover scale. They don't grow while motion is reduced.
    pub fn hover_grow() -> f32 {
        if reduced() { 1.0 } else { super::size::HOVER_GROW }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    /// Interface size as a fraction, 1.0 at 100%.
    pub zoom: f32,
    pub accent: Srgb,
    pub on_accent: Srgb,
    pub accent_soft: Srgb,
    /// The window is translucent, so the page and sidebar let the blurred desktop through.
    pub translucent: bool,
}

const ON_ACCENT_DARK: u32 = 0x0b1220;

impl Theme {
    pub fn new(accent_rgb: u32, zoom: f32, translucent: bool) -> Self {
        let accent = Srgb::hex(accent_rgb);
        Self { zoom, accent, on_accent: Self::contrast_text(accent), accent_soft: accent.mix(Srgb::hex(0xffffff), 0.55), translucent }
    }

    /// White or the dark ink, whichever has more APCA contrast on the accent.
    fn contrast_text(accent: Srgb) -> Srgb {
        let (dark, white) = (Srgb::hex(ON_ACCENT_DARK), Srgb::hex(0xffffff));
        if apca_lc(white, accent).abs() > apca_lc(dark, accent).abs() { white } else { dark }
    }

    /// A token length on screen. Not rounded, since GPUI snaps to device pixels when painting and
    /// column positions shouldn't accumulate rounding.
    pub fn s(&self, token: f32) -> Pixels {
        px(token * BASE_SCALE * self.zoom)
    }

    /// Token units per screen pixel, for converting widths back to tokens.
    pub fn token_scale(&self) -> f32 {
        BASE_SCALE * self.zoom
    }

    /// Text size on screen. `TEXT_MD` is 14px at 100%.
    pub fn text(&self, token: f32) -> Pixels {
        self.s(token)
    }

    /// The page behind everything, letting the desktop through when translucent.
    pub fn bg(&self) -> Rgba {
        let bg = rgb(BG);
        if self.translucent { Rgba { a: BG_WINDOW_ALPHA, ..bg } } else { bg }
    }
    pub fn panel(&self) -> Rgba {
        rgb(PANEL)
    }
    pub fn panel_2(&self) -> Rgba {
        rgb(PANEL_2)
    }
    /// Text and icon colors, at 90%, 68% and 57% white.
    pub fn t1(&self) -> Rgba {
        rgba(0xffffffe6)
    }
    pub fn t2(&self) -> Rgba {
        rgba(0xffffffad)
    }
    pub fn t3(&self) -> Rgba {
        rgba(0xffffff91)
    }
    /// Pure white, for page and item titles and selected items.
    pub fn t_em(&self) -> Rgba {
        rgb(0xffffff)
    }
    /// Sidebar row ink, and section headings.
    pub fn t_nav(&self) -> Rgba {
        self.t2()
    }
    /// Text and icons over artwork or the tinted player. Also switch knobs, slider thumbs, and text
    /// on a danger fill.
    pub fn t_on_art(&self) -> Hsla {
        gpui::white()
    }
    pub fn knob(&self) -> Hsla {
        gpui::white()
    }
    pub fn on_danger(&self) -> Hsla {
        gpui::white()
    }
    pub fn t_section(&self) -> Rgba {
        self.t3()
    }
    /// The bottom bar's title, ink and track. The bar is neutral, never the cover tint.
    pub fn bar_title(&self) -> Rgba {
        self.t1()
    }
    pub fn bar_ink(&self) -> Rgba {
        self.t2()
    }
    pub fn bar_track(&self) -> Rgba {
        rgba(0xffffff38)
    }
    /// The unplayed track in the dock and expanded player.
    pub fn player_track(&self) -> Rgba {
        rgba(0x00000033)
    }
    /// The player's Lyrics, Artwork and Queue switcher over the cover tint.
    pub fn switcher_glass(&self) -> Rgba {
        rgba(0xffffff1f)
    }
    pub fn row(&self) -> Rgba {
        rgba(0xffffff12)
    }
    pub fn hover(&self) -> Rgba {
        rgba(0xffffff1a)
    }
    /// What a small control's ink brightens to on hover. The tinted player uses `player_fg` instead.
    pub fn hover_ink(&self) -> Hsla {
        self.t1().into()
    }
    /// A hovered button's fill while pressed.
    pub fn press(&self) -> Rgba {
        rgba(0xffffff29)
    }
    /// A downloaded collection's filled mark, and a download ring as it fills.
    pub fn downloaded(&self) -> Hsla {
        self.accent_hsla()
    }
    /// The arrow on the filled download mark.
    pub fn downloaded_ink(&self) -> Hsla {
        self.on_accent.hsla(1.0)
    }
    /// The ring a download fills.
    pub fn download_track(&self) -> Rgba {
        rgba(0xffffff38)
    }
    /// A sync's bar, the accent over the download track.
    pub fn sync_fill(&self) -> Hsla {
        self.accent_hsla()
    }
    pub fn sync_track(&self) -> Rgba {
        self.download_track()
    }
    /// The ring and arrow of a waiting download, such as when metered, offline or queued.
    pub fn download_paused(&self) -> Rgba {
        self.t3()
    }
    pub fn line(&self) -> Rgba {
        rgba(0xffffff14)
    }
    pub fn glass_on(&self) -> Rgba {
        rgba(0xffffff1f)
    }
    /// A filter chip that's on, a light fill with dark ink.
    pub fn chip_on(&self) -> Rgba {
        rgb(0xffffff)
    }
    pub fn on_chip(&self) -> Rgba {
        rgb(0x111111)
    }
    /// A glass button's fill while hovered.
    pub fn glass_hover(&self) -> Rgba {
        rgba(0x5c5c6285)
    }
    /// A glass control's fill while pressed or open.
    pub fn glass_press(&self) -> Rgba {
        rgba(0x70707694)
    }
    /// The 1px divider between controls inside a glass pill.
    pub fn glass_divider(&self) -> Rgba {
        rgba(0xffffff38)
    }
    /// The surface of every floating panel, over the float blur. `float_fill` stands in without
    /// backdrop blur.
    pub fn float_glass(&self) -> Rgba {
        rgba(0x26262ad1)
    }
    pub fn float_fill(&self) -> Rgba {
        rgba(0x26262af5)
    }
    /// Dialogs and toasts, a little more solid than a menu. `dialog_fill` stands in without blur.
    pub fn dialog_glass(&self) -> Rgba {
        rgba(0x26262ae0)
    }
    pub fn dialog_fill(&self) -> Rgba {
        rgba(0x26262afa)
    }
    /// The scrim behind a dialog and the sidebar drawer.
    pub fn scrim(&self) -> Rgba {
        rgba(0x00000073)
    }
    /// A selected song row, the accent at 30%.
    pub fn row_selected(&self) -> Hsla {
        self.accent.hsla(0.3)
    }
    /// The soft slot where dragged rows will land, the lifted row's card, and the second card that
    /// peeks out under a multi-row drag.
    pub fn drag_slot(&self) -> Rgba {
        rgba(0xffffff0d)
    }
    pub fn drag_slot_edge(&self) -> Rgba {
        rgba(0xffffff12)
    }
    pub fn drag_card(&self) -> Rgba {
        rgb(PANEL_2)
    }
    pub fn drag_card_edge(&self) -> Rgba {
        rgba(0xffffff1f)
    }
    pub fn drag_card_under(&self) -> Rgba {
        rgb(0x34343a)
    }
    pub fn drag_card_under_edge(&self) -> Rgba {
        rgba(0xffffff14)
    }
    /// A drop target under dragged songs, filled with the accent at 24% and ringed in the accent.
    pub fn drop_target(&self) -> Hsla {
        self.accent.hsla(0.24)
    }
    pub fn drop_target_edge(&self) -> Hsla {
        self.accent_hsla()
    }
    /// A queue row hovered or selected over the tinted player.
    pub fn on_art_hover(&self) -> Rgba {
        rgba(0xffffff1f)
    }
    pub fn on_art_selected(&self) -> Rgba {
        rgba(0xffffff38)
    }
    /// Where artwork hasn't arrived yet, over the player or the bar's glass.
    pub fn on_art_placeholder(&self) -> Rgba {
        rgba(0xffffff14)
    }
    /// The player switcher's selected segment and its edge.
    pub fn switcher_on(&self) -> Rgba {
        rgba(0xffffff33)
    }
    pub fn switcher_edge(&self) -> Rgba {
        rgba(0xffffff38)
    }
    /// The panel glass of the sidebar and the Settings and Equalizer cards, so page tints reach
    /// behind it without a hard edge. When translucent it's a white overlay letting through as much
    /// of the blurred desktop as the page.
    pub fn panel_glass(&self) -> Rgba {
        if self.translucent { rgba(0xffffff07) } else { rgba(0x1f1f21b3) }
    }
    /// The panel glass for pages whose tint or glow reaches behind the sidebar. A white lift over a
    /// bright photo washes out the text, so these always get the dark glass.
    pub fn sidebar_glass(&self) -> Rgba {
        rgba(0x1f1f21b3)
    }
    /// The solid sidebar color a pinned header fades scrolled content into.
    pub fn sidebar_fade(&self) -> Rgba {
        rgb(0x1d1d1e)
    }
    /// Glass over the page, nearly opaque so scrolled rows stay unreadable behind the pills.
    pub fn glass_fill(&self) -> Rgba {
        rgba(0x27272afa)
    }
    /// Glass for pills that blur what passes under them.
    pub fn glass_frost_fill(&self) -> Rgba {
        rgba(0x3a3a3e75)
    }
    /// The bar glass over the page, for renderers without backdrop blur.
    pub fn bar_fill(&self) -> Rgba {
        rgba(0x2f2f33fa)
    }
    /// The bar glass, over the bar's backdrop blur.
    pub fn bar_frost_fill(&self) -> Rgba {
        rgba(0x2e2e32b3)
    }
    pub fn accent_hsla(&self) -> Hsla {
        self.accent.hsla(1.0)
    }
    pub fn accent_soft_hsla(&self) -> Hsla {
        self.accent_soft.hsla(1.0)
    }
}
