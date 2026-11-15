import AppKit
import Synchronization
import SwiftUI

/// A color usable from both SwiftUI and AppKit.
struct RGBA: Equatable, Sendable {
    var r, g, b, a: Double

    init(_ r: Double, _ g: Double, _ b: Double, _ a: Double = 1) {
        self.r = r; self.g = g; self.b = b; self.a = a
    }

    /// `#rrggbb`, with or without the leading `#`.
    init?(hex: String) {
        let s = hex.hasPrefix("#") ? String(hex.dropFirst()) : hex
        guard s.count == 6, let v = UInt32(s, radix: 16) else { return nil }
        self.init(Double((v >> 16) & 255) / 255, Double((v >> 8) & 255) / 255, Double(v & 255) / 255)
    }

    /// CSS `rgb(r g b / a)` with 0-255 channels.
    static func css(_ r: Double, _ g: Double, _ b: Double, _ a: Double = 1) -> RGBA {
        RGBA(r / 255, g / 255, b / 255, a)
    }

    /// CSS `hsl(h s% l%)`.
    static func hsl(_ h: Double, _ s: Double, _ l: Double) -> RGBA {
        let hue = h.truncatingRemainder(dividingBy: 360) + (h < 0 ? 360 : 0)
        let c = (1 - abs(2 * l - 1)) * s
        let x = c * (1 - abs((hue / 60).truncatingRemainder(dividingBy: 2) - 1))
        let m = l - c / 2
        let (r, g, b): (Double, Double, Double)
        switch hue {
        case ..<60: (r, g, b) = (c, x, 0)
        case ..<120: (r, g, b) = (x, c, 0)
        case ..<180: (r, g, b) = (0, c, x)
        case ..<240: (r, g, b) = (0, x, c)
        case ..<300: (r, g, b) = (x, 0, c)
        default: (r, g, b) = (c, 0, x)
        }
        return RGBA(r + m, g + m, b + m)
    }

    var color: Color { Color(.sRGB, red: r, green: g, blue: b, opacity: a) }
    var ns: NSColor { NSColor(srgbRed: r, green: g, blue: b, alpha: a) }

    func opacity(_ value: Double) -> RGBA { RGBA(r, g, b, value) }

    /// CSS `color-mix(in srgb, self amount, other)`.
    func mixed(with other: RGBA, amount: Double) -> RGBA {
        RGBA(r * amount + other.r * (1 - amount), g * amount + other.g * (1 - amount), b * amount + other.b * (1 - amount), a * amount + other.a * (1 - amount))
    }

    var luminance: Double {
        func channel(_ c: Double) -> Double { c <= 0.03928 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4) }
        return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
    }

    func contrast(against other: RGBA) -> Double {
        let (a, b) = (luminance, other.luminance)
        return (max(a, b) + 0.05) / (min(a, b) + 0.05)
    }

    /// APCA (SAPC/APCA-W3 0.0.98G) constants for perceptual lightness contrast. WCAG 2 ratios favor black on
    /// mid-tones, where white reads better. GPUI's color.rs uses the same constants.
    private enum APCA {
        static let trc = 2.4, rCo = 0.2126729, gCo = 0.7151522, bCo = 0.072175
        static let blkThrs = 0.022, blkClmp = 1.414
        static let normBG = 0.56, normTXT = 0.57, revTXT = 0.62, revBG = 0.65
        static let scale = 1.14, loOffset = 0.027, loClip = 0.1, deltaYmin = 0.0005
    }

    /// APCA's screen luminance, with its soft clamp near black.
    private var apcaY: Double {
        let y = APCA.rCo * pow(r, APCA.trc) + APCA.gCo * pow(g, APCA.trc) + APCA.bCo * pow(b, APCA.trc)
        return y < APCA.blkThrs ? y + pow(APCA.blkThrs - y, APCA.blkClmp) : y
    }

    /// APCA lightness contrast (Lc) of this color as text on `background`: positive for dark on light,
    /// negative for light on dark.
    func apcaLc(on background: RGBA) -> Double {
        let (t, b) = (apcaY, background.apcaY)
        if abs(b - t) < APCA.deltaYmin { return 0 }
        if b > t {
            let s = (pow(b, APCA.normBG) - pow(t, APCA.normTXT)) * APCA.scale
            return s < APCA.loClip ? 0 : (s - APCA.loOffset) * 100
        }
        let s = (pow(b, APCA.revBG) - pow(t, APCA.revTXT)) * APCA.scale
        return s > -APCA.loClip ? 0 : (s + APCA.loOffset) * 100
    }
}

/// The design tokens, with the same names and meanings as GPUI's `theme.rs`. Sizes are CSS pixels rendered at
/// 87.5%, so every size goes through `px`: 16 becomes 14 points.
///
/// `zoom` is the Interface size setting, 1 at 100%. `RootView` rebuilds the page when it changes.
enum T {
    static var zoom: CGFloat = 1
    static var scale: CGFloat { 0.875 * zoom }
    static func px(_ value: CGFloat) -> CGFloat { value * scale }

    static let bg = RGBA(hex: "171718")!
    static let panel = RGBA(hex: "1f1f21")!
    static let panelGlass = RGBA.css(31, 31, 33, 0.7)
    /// The page color inside the sidebar's glass, Mac only. The system glass lifts itself much more while the window
    /// is inactive, so this keeps the sidebar a few levels above the window color either way.
    static let panelGlassVeil = bg.opacity(0.25)
    static let panelGlassVeilInactive = bg.opacity(0.6)
    static let panel2 = RGBA(hex: "2a2a2d")!
    static let row = RGBA.css(255, 255, 255, 0.07)
    static let danger = RGBA(hex: "e5484d")!
    static let hover = RGBA.css(255, 255, 255, 0.10)
    /// Small controls brighten their ink on hover instead of taking a fill, and dim when pressed. Play and skip
    /// grow instead, and shrink when pressed.
    static let hoverInk = t1
    static let pressDim = 0.7
    /// A disabled control keeps its shape and colors at this opacity.
    static let disabledOpacity = 0.4
    /// The player's mode buttons at rest, and disabled.
    static let modeRest = 0.62
    static let modeDisabled = 0.35
    /// Play and skip don't grow while motion is reduced (`Motion`).
    nonisolated static var hoverGrow: CGFloat { Motion.isReduced ? 1 : 1.08 }
    static let pressShrink: CGFloat = 0.94
    /// A hovered button while it's held down.
    static let press = RGBA.css(255, 255, 255, 0.16)
    static let line = RGBA.css(255, 255, 255, 0.08)

    /// Text and icon ink: 90%, 68% and 57% white.
    static let t1 = RGBA.css(255, 255, 255, 0.90)
    static let t2 = RGBA.css(255, 255, 255, 0.68)
    static let t3 = RGBA.css(255, 255, 255, 0.57)
    /// Pure white for titles, the selected sidebar row, tab and segment, and the sorted column.
    static let tEm = RGBA(1, 1, 1)
    /// Sidebar rows, and section headings.
    static let tNav = t2
    static let tSection = t3
    /// Text and icons over artwork or the cover-tinted player, and slider thumbs.
    static let tOnArt = RGBA(1, 1, 1)
    static let knob = RGBA(1, 1, 1)
    /// The bottom bar is neutral, never the cover tint: its title, its glyphs and secondary text, and its
    /// unplayed track.
    static let barTitle = t1
    static let barInk = t2
    static let barTrack = RGBA.css(255, 255, 255, 0.22)
    /// The unplayed track in the dock and the expanded player.
    static let playerTrack = RGBA.css(0, 0, 0, 0.2)
    /// The expanded player's text scrim: wide, soft and light so it never reads as a box.
    static var glowScrimReach: CGFloat { px(96) }
    static var glowScrimBlur: CGFloat { px(64) }
    static let glowScrimMax = 0.32

    static let glass = RGBA.css(58, 58, 62, 0.46)
    static let glassOn = RGBA.css(255, 255, 255, 0.12)
    static let glassEdge = RGBA.css(255, 255, 255, 0.16)
    /// A flat glass fill under the pointer and held down, and the line between segments of a glass group.
    static let glassHover = RGBA.css(92, 92, 98, 0.52)
    static let glassPress = RGBA.css(112, 112, 118, 0.58)
    static let glassDivider = RGBA.css(255, 255, 255, 0.22)
    /// Behind a dialog and the sidebar drawer.
    static let scrim = RGBA(0, 0, 0, 0.45)
    /// Blur radii are CSS blur() values in tokens. Liquid Glass sets its own blur, so the bar's frost comes from its
    /// tint.
    static var glassBlur: CGFloat { px(24) }
    /// Menus, popovers and the bar's volume panel. Dialogs and toasts are a little more solid.
    static let floatGlass = RGBA.css(38, 38, 42, 0.82)
    static var floatBlur: CGFloat { px(30) }
    static let dialogGlass = RGBA.css(38, 38, 42, 0.88)
    /// Download marks: a filling ring's track, a waiting download, the mark on rows and cards, the glyph in buttons,
    /// and the room before a mark.
    static let downloadTrack = RGBA.css(255, 255, 255, 0.22)
    static var downloadPaused: RGBA { t3 }
    static var downloadMark: CGFloat { px(14) }
    static var downloadGlyph: CGFloat { px(20) }
    static var downloadMarkGap: CGFloat { px(6) }
    /// The room above each queue section heading.
    static var queueSectionGap: CGFloat { px(14) }
    /// Placeholder rows under the Autoplay heading while its songs are being found.
    static let queueFindingRows = 3
    /// A placeholder row's two text lines as quiet blocks: height, gap, and widths as fractions of the row.
    static var rowPlaceholderLine: CGFloat { px(9) }
    static var rowPlaceholderGap: CGFloat { px(8) }
    static let rowPlaceholderTitleW: CGFloat = 0.55
    static let rowPlaceholderArtistW: CGFloat = 0.35
    /// The square X that removes a queue row.
    static var queueRemoveW: CGFloat { px(28) }
    /// The queue headings' icon buttons (Clear, Shuffle): glyph size and gap.
    static var queueHeadIcon: CGFloat { px(18) }
    /// Listening Now rows: the cover beside each person, and the live mark's three accent bars.
    static var personArt: CGFloat { px(30) }
    static var liveMarkH: CGFloat { px(10) }
    static var liveBarW: CGFloat { px(2) }
    static var liveBarGap: CGFloat { px(1.5) }
    /// A Spotify friend's row: the friend mark on the avatar's corner, and initials when there's no avatar.
    static var friendMark: CGFloat { px(13) }
    static var friendMarkGlyph: CGFloat { px(8) }
    static var friendInitials: CGFloat { px(11) }
    static var queueHeadGap: CGFloat { px(8) }
    /// A dialog's width, padding, gaps, and button padding.
    static var dialogW: CGFloat { px(420) }
    /// The sheet variant of a dialog (Get Info) and its info rows.
    static var sheetW: CGFloat { px(560) }
    static var sheetMaxH: CGFloat { px(780) }
    static var infoArt: CGFloat { px(88) }
    static var infoLabelW: CGFloat { px(148) }
    static var infoRowPadY: CGFloat { px(9) }
    static var infoRowPadX: CGFloat { px(14) }
    static var infoSectionGap: CGFloat { px(18) }
    /// The cover sheet (an artist's About). Focus and fade positions are fractions of the picture's height.
    static var sheetCoverH: CGFloat { px(420) }
    static var sheetCoverOverlap: CGFloat { px(120) }
    static let sheetCoverFocusY: CGFloat = 0.22
    static let sheetCoverFadeFrom: CGFloat = 0.45
    static var sheetSectionGap: CGFloat { px(22) }
    static var sheetLabelGap: CGFloat { px(8) }
    static var dialogPad: CGFloat { px(22) }
    static var dialogGap: CGFloat { px(14) }
    static var dialogActionsGap: CGFloat { px(10) }
    static var dialogActionsTop: CGFloat { px(4) }
    /// A text field's side padding, and the accent ring outside a focused field.
    static var fieldPad: CGFloat { px(15) }
    static let focusRingWidth: CGFloat = 2
    /// Every floating panel's corners.
    static var rFloat: CGFloat { px(16) }
    /// A menu's submenu (Add to Playlist). Mac menus are native, so only the width is used, for the filter field.
    static var submenuW: CGFloat { px(260) }
    /// The anchored popover. Its content scrolls past the max height.
    static var popW: CGFloat { px(380) }
    static var popMaxH: CGFloat { px(440) }
    /// Song filter chips, the filter popover and its fields, and the gaps around the chip row.
    static var chipH: CGFloat { px(32) }
    static var chipPad: CGFloat { px(13) }
    static var chipGlyph: CGFloat { px(12) }
    static var chipGap: CGFloat { px(6) }
    static var filterRowGap: CGFloat { px(16) }
    static var chipValueMaxW: CGFloat { px(180) }
    static var filterPopW: CGFloat { px(300) }
    static var filterListMaxH: CGFloat { px(300) }
    static var numberFieldW: CGFloat { px(76) }
    /// The smart playlist editor's pills, rows, group indent, sort rows and limit field.
    static var smartFieldW: CGFloat { px(220) }
    static var smartOpW: CGFloat { px(175) }
    static var smartRowGap: CGFloat { px(8) }
    static var smartGroupIndent: CGFloat { px(14) }
    static var smartSortW: CGFloat { px(220) }
    static var smartSortLabelW: CGFloat { px(64) }
    static var smartLimitW: CGFloat { px(140) }
    /// A filter chip that's on: a light fill with dark ink.
    static let chipOn = RGBA.css(255, 255, 255, 1)
    static let onChip = RGBA.css(17, 17, 17, 1)
    /// The hue slider's saturation and lightness, those of the default accent, hsl(218 88.6% 65.7%).
    static let accentS = 0.886
    static let accentL = 0.657
    /// Mac only: room above the sidebar's first row, so the scroll view's soft top edge doesn't clip its highlight.
    static var sidebarListTop: CGFloat { px(6) }
    /// How far a toast floats above the bottom of the content column.
    static var toastGap: CGFloat { px(32) }
    /// A toast's height and side padding. One with an action keeps the height, with a small button at the right.
    static var toastH: CGFloat { px(42) }
    static var toastPad: CGFloat { px(18) }
    /// Between an action button's icon and its label.
    static var buttonGap: CGFloat { px(8) }
    /// The small glass button, such as a toast's Undo.
    static var buttonSmallH: CGFloat { px(30) }
    static var buttonSmallPad: CGFloat { px(12) }
    /// How long a toast stays. One with an action stays longer, and hovering keeps it.
    static let toastSeconds = 3.0
    static let toastActionSeconds = 5.0
    /// The player's Lyrics, Artwork and Queue switcher over the cover tint.
    static let switcherGlass = RGBA.css(255, 255, 255, 0.12)
    static let switcherEdge = RGBA.css(255, 255, 255, 0.22)
    static let switcherOn = RGBA.css(255, 255, 255, 0.20)
    /// A queue row hovered and selected over the tinted player.
    static let onArtHover = RGBA.css(255, 255, 255, 0.12)
    static let onArtSelected = RGBA.css(255, 255, 255, 0.22)
    /// Where artwork or a row hasn't arrived yet over the player.
    static let onArtPlaceholder = RGBA.css(255, 255, 255, 0.08)
    /// What the player gradients darken toward: the expanded player's corners, the dock's bottom, and the fade under
    /// the dock's buttons.
    static let playerShadeStart = RGBA(hex: "081018")!
    static let playerShadeEnd = RGBA(hex: "060a14")!
    static let dockShade = RGBA(hex: "0a0e16")!
    static let dockHeadShade = RGBA(0, 0, 0, 0.34)
    /// How much of the tint stays where the dock and players darken toward a shade.
    static let playerShadeMix = 0.72
    /// The dark fill under window controls drawn over a cover (the large mini player).
    static let windowButtonOnArt = RGBA(0, 0, 0, 0.22)
    static let windowButtonOnArtHover = RGBA(0, 0, 0, 0.35)
    /// The share of the room above the current lyric line that stays clear before the mask fades in, so lines are
    /// gone before they reach the player's buttons.
    static let lyricTopClear = 0.5
    /// Synced lyrics by distance d from the current line: opacity `max(min, near - d * step)` and blur
    /// `min(max, near + d * step)`.
    static let lyricDimNear = 0.46
    static let lyricDimStep = 0.07
    static let lyricDimMin = 0.14
    static let lyricHover = 0.9
    static let lyricBlurNear: CGFloat = 1.2
    static let lyricBlurStep: CGFloat = 0.9
    static let lyricBlurMax: CGFloat = 5.5
    static let lyricGlow = Shadow(color: RGBA.css(255, 255, 255, 0.28), blur: 16)
    static let lyricUnsung = RGBA.css(255, 255, 255, 0.58)
    /// The bottom bar: more frosted than a popover.
    static let barGlass = RGBA.css(46, 46, 50, 0.70)
    static var barBlur: CGFloat { px(44) }
    static var sidebarBlur: CGFloat { px(38) }
    static var edgeBlur: CGFloat { px(24) }
    /// Soft copies of artwork: the dock's reflection, the hero wash, the page glow and the player's backdrop.
    static var reflectBlur: CGFloat { px(32) }
    /// The reflection's opacity and the soft copies' saturation.
    static let reflectOpacity = 0.52
    static let reflectSaturation = 1.2
    /// An artist's name over the glow or a cover.
    static let heroTitleShadow = Shadow(color: RGBA(0, 0, 0, 0.4), blur: 22, y: 2)
    /// The hero wash behind Discover.
    static var heroGlowBlur: CGFloat { px(44) }
    static let heroGlowSaturation = 1.3
    static let heroGlowOpacity = 0.6
    /// The page glow and the expanded player's glow. Brightness multiplies, as CSS `brightness()` does.
    static var pageGlowBlur: CGFloat { px(90) }
    static let pageGlowSaturation = 1.2
    static let pageGlowBrightness = 0.72
    static let pageGlowOpacity = 0.72
    static var playerGlowBlur: CGFloat { px(110) }
    static let playerGlowSaturation = 1.3
    static let playerGlowBrightness = 0.82
    static let playerGlowOpacity = 0.55
    /// The bar glow's strength at its brightest.
    static let barGlowOpacity = 0.24

    /// The system blur behind a translucent window, kept dark by the app's forced dark appearance. `.sidebar` is what
    /// Finder's sidebar uses, for its strong, soft blur of the desktop.
    static let windowMaterial = NSVisualEffectView.Material.sidebar
    /// The page color over that blur: secondary text keeps 4:1 contrast even over pure white.
    static let windowTint = bg.opacity(0.74)
    /// The window server's blur radius behind the translucent window, in its own units.
    static let windowBlurRadius = 80

    /// A CSS box shadow: `blur` is the CSS blur length in tokens (SwiftUI's radius is half) and `y` the offset.
    /// SwiftUI has no spread, so negative spreads are left out.
    struct Shadow {
        var color: RGBA
        var blur: CGFloat
        var x: CGFloat = 0
        var y: CGFloat = 0
    }
    /// Floating panels, dialogs, the bar and the sidebar drawer.
    static let shadowFloat = Shadow(color: RGBA(0, 0, 0, 0.7), blur: 50, y: 18)
    /// The top bar's glass pills.
    static let shadowPill = Shadow(color: RGBA(0, 0, 0, 0.5), blur: 26, y: 8)
    /// A page's big cover.
    static let shadowArt = Shadow(color: RGBA(0, 0, 0, 0.7), blur: 48, y: 20)
    /// A row lifted by a drag, and the dock over the page.
    static let shadowLift = Shadow(color: RGBA(0, 0, 0, 0.45), blur: 30, y: 12)
    static let shadowDock = Shadow(color: RGBA(0, 0, 0, 0.31), blur: 22, x: -11)
    /// The expanded player's cover.
    static let shadowPlayerArt = Shadow(color: RGBA(0, 0, 0, 0.6), blur: 70, y: 30)
    /// The count badge on a multi-row drag.
    static let shadowBadge = Shadow(color: RGBA(0, 0, 0, 0.35), blur: 12, y: 4)
    /// A sidebar row being dragged, slider thumbs, and the EQ graph's handles.
    static let shadowGhost = Shadow(color: RGBA(0, 0, 0, 0.4), blur: 18, y: 6)
    static let shadowKnob = Shadow(color: RGBA(0, 0, 0, 0.4), blur: 4, y: 1)
    static let shadowKnobHue = Shadow(color: RGBA(0, 0, 0, 0.5), blur: 5, y: 1)
    static let shadowEqHandle = Shadow(color: RGBA(0, 0, 0, 0.45), blur: 8, y: 2)
    /// The play glyph over a thumbnail.
    static let hoverPlayShadow = Shadow(color: RGBA(0, 0, 0, 0.6), blur: 4, y: 1)

    /// Drag feedback: the empty slot where rows will land, and the lifted card with a second one peeking under it.
    /// The table's AppKit card lets a little of the rows show through.
    static let dragSlot = RGBA.css(255, 255, 255, 0.05)
    static let dragSlotEdge = RGBA.css(255, 255, 255, 0.07)
    static let dragCard = panel2.opacity(0.9)
    static let dragCardEdge = RGBA.css(255, 255, 255, 0.12)
    static let dragCardUnder = RGBA(hex: "34343a")!.opacity(0.86)
    static let dragCardUnderEdge = RGBA.css(255, 255, 255, 0.08)
    /// Songs dragged out of their list: the card that follows the pointer, how dim the rows left behind go, and the
    /// drop target's accent ring, `dropPad` past a target without its own box.
    static let dragOutCardW = px(280)
    /// The card's offset below and right of the pointer.
    static let dragOutOffsetX = px(14)
    static let dragOutOffsetY = px(12)
    static let dragOutDim = 0.4
    static let dropRing = px(2)
    static let dropPad = px(8)

    /// The EQ graph: grid lines, the 0 dB line, a handle's ring and the curve's area.
    static let eqGrid = RGBA.css(255, 255, 255, 0.09)
    static let eqGridBand = RGBA.css(255, 255, 255, 0.04)
    static let eqGridZero = RGBA.css(255, 255, 255, 0.26)
    static let eqHandleRing = RGBA(0, 0, 0, 0.25)
    static let eqAreaOpacity = 0.14
    /// Status boxes, problem fills and icons, and the thin ring round an invalid field or rule value.
    static let statusFill = RGBA.css(255, 255, 255, 0.06)
    static let dangerFill = danger.opacity(0.14)
    static let dangerInk = danger.mixed(with: RGBA(1, 1, 1), amount: 0.7)
    static let invalidRing = RGBA.css(255, 120, 120, 0.7)
    /// A thin ring, so a value with a problem is marked quietly.
    static let invalidRingWidth: CGFloat = 1
    static var statusPadY: CGFloat { px(11) }
    static var statusPadX: CGFloat { px(14) }
    static var statusGap: CGFloat { px(12) }
    /// A one-time question inside a panel (`PromptCard`): its padding and the gap between its parts.
    static var promptCardPad: CGFloat { px(16) }
    static var promptCardGap: CGFloat { px(10) }

    /// The brand: the dark app icon's gradient behind the sign-in screen, and the lockup of the mark and "navia" in
    /// Mona Sans 750 / width 125. Sizes are fractions of the mark's ink and match GPUI's `theme.rs`.
    static let brandTop = RGBA(hex: "3a3a40")!
    static let brandBottom = RGBA(hex: "0b0b0d")!
    static let brandWordWeight: CGFloat = 750
    static let brandWordWidth: CGFloat = 125
    static let brandWordTracking: CGFloat = -0.015
    nonisolated static let brandMarkStroke: CGFloat = 4.4
    static let brandWordRatio: CGFloat = 1.1
    static let brandGapRatio: CGFloat = 0.3

    /// The sign-in screen's form, card and lockup, and a labeled field's gaps.
    static var connectW: CGFloat { px(420) }
    static var connectGap: CGFloat { px(16) }
    static var connectHeadingGap: CGFloat { px(8) }
    static var connectCardPad: CGFloat { px(28) }
    static var connectMark: CGFloat { px(55) }
    static var fieldLabelGap: CGFloat { px(7) }
    static var fieldMessageGap: CGFloat { px(7) }
    static var fieldInset: CGFloat { px(3) }

    /// The explicit "E" mark.
    static let explicitFill = RGBA.css(255, 255, 255, 0.22)
    static let explicitInk = RGBA(0, 0, 0, 0.82)
    /// The tint when an album has none.
    static let tintFallback = RGBA(hex: "34506b")!

    /// Timing. Every hover change takes `hoverIn` going on and `hoverOut` going off, and a press `hoverPress`, all on
    /// the `hoverEase` cubic-bezier. The same values as GPUI.
    nonisolated static let hoverIn = 0.1
    nonisolated static let hoverOut = 0.24
    nonisolated static let hoverPress = 0.1
    nonisolated static let hoverEase: (Double, Double, Double, Double) = (0.2, 0.7, 0.2, 1)
    /// Controls an area's hover reveals stay this long after the pointer leaves, then fade over `hoverOut`.
    nonisolated static let revealGrace = 0.3
    nonisolated static let fast = 0.18
    nonisolated static let med = 0.34
    nonisolated static let dragSlide = 0.17
    nonisolated static let tintFade = 0.32
    nonisolated static let shelfScroll = 0.3
    /// Every animation comes from here, and each is nil while motion is reduced, so views never check the setting.
    /// Things that show progress over time don't use these and keep updating.
    nonisolated static func ease(_ duration: Double) -> Animation? { Motion.isReduced ? nil : .timingCurve(0.2, 0.7, 0.2, 1, duration: duration) }
    nonisolated static func easeOut(_ duration: Double) -> Animation? { Motion.isReduced ? nil : .easeOut(duration: duration) }
    nonisolated static func easeInOut(_ duration: Double) -> Animation? { Motion.isReduced ? nil : .easeInOut(duration: duration) }
    /// A hover fill, ink or grow: in quickly, out gently, on the shared hover curve.
    nonisolated static func hover(_ on: Bool) -> Animation? { hoverCurve(on ? hoverIn : hoverOut) }
    /// Controls an area's hover reveals: in like a hover, out after `revealGrace`.
    nonisolated static func reveal(_ on: Bool) -> Animation? { on ? hover(true) : hover(false)?.delay(revealGrace) }
    /// A press going down (`hoverPress`), or coming back up (`hoverIn`).
    nonisolated static func press(_ down: Bool) -> Animation? { hoverCurve(down ? hoverPress : hoverIn) }
    nonisolated static func hoverCurve(_ duration: Double) -> Animation? {
        Motion.isReduced ? nil : .timingCurve(hoverEase.0, hoverEase.1, hoverEase.2, hoverEase.3, duration: duration)
    }
    /// Runs `changes` in an AppKit animation group, or at once while motion is reduced.
    static func animate(_ duration: Double, _ changes: () -> Void) {
        NSAnimationContext.runAnimationGroup { context in
            context.duration = Motion.isReduced ? 0 : duration
            context.timingFunction = CAMediaTimingFunction(name: .easeOut)
            changes()
        }
    }
    /// AppKit's own animations, such as the table opening a gap under a drag: instant while motion is reduced.
    static func appKitMotion<R>(_ body: () -> R) -> R {
        guard Motion.isReduced else { return body() }
        NSAnimationContext.beginGrouping()
        NSAnimationContext.current.duration = 0
        NSAnimationContext.current.allowsImplicitAnimation = false
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        defer {
            CATransaction.commit()
            NSAnimationContext.endGrouping()
        }
        return body()
    }
    /// Whether a canceled table drag slides its card back to the rows.
    static var dragSlidesBack: Bool { !Motion.isReduced }
    /// A spinner's clock. It stands still while motion is reduced.
    static var spinSchedule: AnimationTimelineSchedule { .animation(paused: Motion.shared.reduced) }
    /// One turn of the busy glyph, in seconds.
    static let spinPeriod: Double = 0.9

    static var textXS: CGFloat { px(14) }
    static var textSM: CGFloat { px(15) }
    static var textMD: CGFloat { px(16) }
    static var textLG: CGFloat { px(18) }
    static var textSection: CGFloat { px(25) }
    static var textFeature: CGFloat { px(21) }
    static var textSubtitle: CGFloat { px(23) }
    static var textTitle: CGFloat { px(42) }

    static var rSM: CGFloat { px(8) }
    static var rMD: CGFloat { px(12) }
    static var rRow: CGFloat { px(13) }
    static var rLG: CGFloat { px(22) }

    /// The page's side margin. A page inside the split view reads its gutter from `PageContext.gutter`.
    static var gutter: CGFloat { px(45) }
    /// The split view (Albums, Artists): the list, its rows and the page beside it. Below `splitMinW` (CSS width)
    /// only the page shows.
    static var splitListW: CGFloat { px(315) }
    static var splitListPad: CGFloat { px(20) }
    static var splitRowH: CGFloat { px(78) }
    static var splitRowPad: CGFloat { px(5) }
    static var splitArt: CGFloat { px(45) }
    static var splitRowGap: CGFloat { px(13) }
    static var splitRowInset: CGFloat { px(6) }
    static var splitLineFrom: CGFloat { px(60) }
    static var splitGlyph: CGFloat { px(26) }
    static var splitGutter: CGFloat { px(20) }
    static var splitEnd: CGFloat { px(25) }
    static let splitMinW: CGFloat = 760
    static var contentTop: CGFloat { px(32) }
    /// The sidebar's default width and drag range, in tokens so a saved width survives a change of interface size.
    static let sidebarDefaultTokens: CGFloat = 245
    static let sidebarMinTokens: CGFloat = 200
    static let sidebarMaxTokens: CGFloat = 360
    /// The invisible strip straddling a resizable edge.
    static var edgeGrabW: CGFloat { px(12) }
    /// The dock's width is in tokens so saved values survive a change of interface size.
    static let dockDefaultTokens: CGFloat = 437
    static let dockMinTokens: CGFloat = 340
    static let dockMaxTokens: CGFloat = 640
    /// What the page keeps beside a dragged dock: the room the default dock leaves at the narrowest docked window.
    static var pageMinW: CGFloat { px(478) }
    static var shelfFade: CGFloat { px(130) }
    static var shelfGap: CGFloat { px(4) }
    /// The margin the sidebar, the dock and the bottom bar keep from the window's edges.
    static var windowInset: CGFloat { px(10) }
    static var windowControlsW: CGFloat { px(76) }
    static var barW: CGFloat { px(1010) }
    static var barH: CGFloat { px(62) }
    /// The player chip over the bar, centered above the play controls.
    static var chipAboveMaxW: CGFloat { px(360) }
    static var chipAboveGap: CGFloat { px(10) }
    /// A code entry such as the AirPlay PIN: digit boxes, the gap between them, and the digit size.
    static var pinBoxW: CGFloat { px(54) }
    static var pinBoxH: CGFloat { px(62) }
    static var pinGap: CGFloat { px(10) }
    static var pinText: CGFloat { textSection }
    /// The AirPlay output badge: glyph to name, and the name's max width beside the bar's title.
    static var outputBadgeGap: CGFloat { px(6) }
    static var outputBadgeMaxW: CGFloat { px(200) }
    static var topbarH: CGFloat { px(76) }
    static var pillH: CGFloat { px(56) }
    static var controlH: CGFloat { px(40) }
    static var iconW: CGFloat { px(38) }
    static var iconH: CGFloat { px(34) }
    static var actionW: CGFloat { px(41) }
    static var thumb: CGFloat { px(42) }
    /// A list page (Downloads, Share Links): max width, gaps and row padding.
    static var listPageW: CGFloat { px(980) }
    static var listPageGap: CGFloat { px(30) }
    static var listRowPadX: CGFloat { px(10) }
    /// A list page's song or collection row.
    static var listRowH: CGFloat { px(68) }
    /// Share links: cover to title, the end-date and visits columns, and the button gap.
    static var shareSubjectGap: CGFloat { px(12) }
    static var shareEndsW: CGFloat { px(170) }
    static var shareVisitsW: CGFloat { px(170) }
    static var shareActionsGap: CGFloat { px(2) }
    static var cardW: CGFloat { px(200) }
    static var cardGap: CGFloat { px(32) }
    static var tileGap: CGFloat { px(20) }
    static var gridCard: CGFloat { px(224) }
    static var gridRowGap: CGFloat { px(34) }
    static var heroArt: CGFloat { px(250) }
    /// The artist header: the photo at most `artistArtMaxRatio` as wide as tall, the gaps around the name, and the
    /// Top Songs and Latest Release columns under it.
    static let artistArtMaxRatio: CGFloat = 1.6
    static var artistHeadGap: CGFloat { px(32) }
    static var artistHeadTitleGap: CGFloat { px(16) }
    static var artistColumnsGap: CGFloat { px(44) }
    static var artistColumnsStackGap: CGFloat { px(24) }
    static var sectionGap: CGFloat { px(42) }
    /// A section heading's gap above its rows or cards, and to a button at its right.
    static var shelfHeadGap: CGFloat { px(16) }
    /// How far below the visible rows a later section counts as near view and gets loaded. Matches GPUI's
    /// `NEAR_VIEW_MARGIN`.
    static var nearViewMargin: CGFloat { 420 }
    /// The gap below a playlist's sort note. The table follows the note directly.
    static var orderNoteGap: CGFloat { px(28) }
    static var miniRowH: CGFloat { px(65) }
    /// The mini player: its compact size and large square, padding, controls, glass, scrim, and starting margin from
    /// the screen corner.
    static var miniW: CGFloat { px(366) }
    static var miniH: CGFloat { px(100) }
    static var miniLarge: CGFloat { px(366) } // Same as miniW, so the large one is square.
    static var miniR: CGFloat { rFloat }
    static var miniPad: CGFloat { px(12) }
    static var miniGap: CGFloat { px(12) }
    static var miniButton: CGFloat { px(34) }
    static var miniChromeButton: CGFloat { px(28) }
    static let miniGlass = 0.86
    static let miniScrimStart = 0.34
    static let miniScrimStrength = 0.92
    static var miniCornerMargin: CGFloat { px(16) }
    /// It resizes between the compact size and `miniMax` square, and stacks (cover behind, body over a scrim) once it's
    /// at least `miniStackH` tall and `miniStackRatio` of its width.
    static var miniMinW: CGFloat { miniW }
    static var miniMinH: CGFloat { miniH }
    static var miniMax: CGFloat { px(640) }
    static var miniStackH: CGFloat { px(220) }
    static let miniStackRatio: CGFloat = 0.6
    /// The band along each edge that resizes it. Corners take squares twice as big.
    static var miniGrip: CGFloat { px(8) }
    /// The mini player's hairline seek bar, and a sync's progress bar and meter width. The sync track is the download
    /// ring's track.
    static var syncBarH: CGFloat { px(3) }
    static var syncMeterW: CGFloat { px(280) }
    static var syncTrack: RGBA { downloadTrack }
    /// With no total, a segment sweeps across the sync bar. A sync that isn't the first shows only after
    /// `syncShowAfter`.
    static let syncSweep: Double = 1.4
    static let syncSweepSpan: Double = 0.3
    static let syncShowAfter: Double = 10
    static var scrubThin: CGFloat { px(3) }
    static var scrubThinKnob: CGFloat { px(10) }
    /// Seek bar and volume thumbs, and how small one starts as it fades in.
    static var scrubKnob: CGFloat { px(12) }
    static let scrubKnobRestScale: CGFloat = 0.6
    static var scrubChipEdge: CGFloat { px(46) }
    static var rXS: CGFloat { px(6) }
    /// How far a card or tile rises under the pointer.
    static var cardLift: CGFloat { px(3) }
    /// The quiet "Mix" chip on a cover the app made.
    static var coverLabelInset: CGFloat { px(8) }
    static var coverLabelPadY: CGFloat { px(2) }
    static var coverLabelPadX: CGFloat { px(8) }
    /// From a card's art to the text under it.
    static var cardTextGap: CGFloat { px(10) }
    /// Artist Radio tiles, as fractions of the tile's width: the name's padding, the seed's photo and its neighbors
    /// behind it, the ring that parts them, and the name's size.
    static let stationPad: CGFloat = 0.06
    static let stationFacesY: CGFloat = 0.46
    static let stationFace: CGFloat = 0.50
    static let stationSideFace: CGFloat = 0.36
    static let stationRing: CGFloat = 0.015
    static let stationSideShift: CGFloat = 0.36
    static let stationSideShiftHover: CGFloat = 0.39
    static let stationName: CGFloat = 0.13
    static let stationNameLong: CGFloat = 0.095

    /// Genre tiles: landscape, with the name inset from the top left and up to three covers fanned from the bottom right.
    static let genreTileRatio: CGFloat = 4.0 / 3.0
    /// The name's inset as fractions of the tile's width.
    static let genreTilePadY: CGFloat = 0.07
    static let genreTilePadX: CGFloat = 0.08
    /// A cover's width as a fraction of the tile's, and how far the front cover's corner sits past the tile's edges.
    static let genreFanSize: CGFloat = 0.46
    static let genreFanRight: CGFloat = -0.05
    static let genreFanBottom: CGFloat = -0.17
    /// One fanned cover: its shift as a fraction of its own size, and its turn. Front to back.
    struct FanCard: Sendable { let x: CGFloat; let y: CGFloat; let degrees: Double }
    static let genreFan = [FanCard(x: 0, y: 0, degrees: 10), FanCard(x: -0.30, y: 0, degrees: -2), FanCard(x: -0.56, y: 0, degrees: -14)]
    static let genreFanHover = [FanCard(x: 0, y: -0.03, degrees: 13), FanCard(x: -0.34, y: -0.02, degrees: -3), FanCard(x: -0.64, y: 0, degrees: -18)]
    static let genreFanSingle = FanCard(x: 0, y: 0, degrees: 6)
    /// A tinted tile darkens toward the covers' corner: the lightness multiplier there and the ramp's angle in CSS degrees.
    static let tileFillShade = 0.7
    static let tileFillAngle = 150.0
    /// The most lightness a tint background may have under light text.
    nonisolated static let tintTextSafeL = 0.47
    /// Dense genre rows: their minimum width, side padding, and cover-to-text gap.
    static var genreChipW: CGFloat { px(250) }
    static var genreRowPad: CGFloat { px(8) }
    static var genreRowGap: CGFloat { px(13) }

    /// 78 CSS pixels, 68.25 points. Whole points keep the 1px separators crisp.
    static var rowH: CGFloat { (68 * zoom).rounded() }
    static var rowCompactH: CGFloat { (38 * zoom).rounded() }

    /// Window widths below which the dock becomes the bar and the sidebar becomes a drawer.
    static var dockMinWindow: CGFloat { px(1180) }
    static var sidebarMinWindow: CGFloat { px(860) }
}

extension Font {
    static func navia(_ size: CGFloat, _ weight: Font.Weight = .regular) -> Font { .system(size: size, weight: weight) }
}

/// Colors the user can change at runtime, saved as `accentHex`.
@Observable final class Theme {
    static let defaultAccent = RGBA(hex: "5a93f5")!

    var accent: RGBA

    init() {
        accent = UserDefaults.standard.string(forKey: "accentHex").flatMap(RGBA.init(hex:)) ?? Theme.defaultAccent
    }

    /// `#rrggbb`, lower case.
    var hex: String {
        String(format: "#%02x%02x%02x", Int((accent.r * 255).rounded()), Int((accent.g * 255).rounded()), Int((accent.b * 255).rounded()))
    }

    func setAccent(hex: String) {
        guard let color = RGBA(hex: hex) else { return }
        accent = color
        UserDefaults.standard.set(hex, forKey: "accentHex")
    }

    /// Matches GPUI's `hue_to_hex`: a fixed saturation and lightness, so any hue works on the dark page.
    func setAccent(hue: Double) {
        let color = RGBA.hsl(hue, T.accentS, T.accentL)
        accent = color
        UserDefaults.standard.set(hex, forKey: "accentHex")
    }

    /// Matches GPUI's `hex_to_hue`.
    var hue: Double {
        let (r, g, b) = (accent.r, accent.g, accent.b)
        let high = max(r, g, b), d = high - min(r, g, b)
        guard d > 0 else { return 0 }
        let h = high == r ? (g - b) / d : high == g ? (b - r) / d + 2 : (r - g) / d + 4
        return (h * 60 + 360).truncatingRemainder(dividingBy: 360)
    }

    /// A selected song row: the accent at 30%.
    var rowSelected: RGBA { accent.opacity(Self.rowSelectedAlpha) }
    static let rowSelectedAlpha = 0.30

    /// A downloaded collection's mark and a filling download ring.
    var downloaded: RGBA { accent }
    /// A sync's bar.
    var syncFill: RGBA { accent }
    var downloadedInk: RGBA { onAccent }

    /// A drop target under dragged songs: the accent at 24%.
    var dropTarget: RGBA { accent.opacity(0.24) }

    /// The current song's title.
    var accentSoft: RGBA { accent.mixed(with: RGBA(1, 1, 1), amount: 0.55) }

    /// White or dark ink, whichever has the larger APCA contrast on the accent.
    var onAccent: RGBA {
        let dark = RGBA(hex: "0b1220")!, white = RGBA(1, 1, 1)
        return abs(white.apcaLc(on: accent)) > abs(dark.apcaLc(on: accent)) ? white : dark
    }
}

extension EnvironmentValues {
    /// True while drawing an offscreen check image, where AppKit-backed views can't render.
    @Entry var isSnapshot = false
    /// The user's colors, for shared views that aren't handed the shell (the sync meter under an empty state).
    @Entry var theme: Theme? = nil
}

/// Reduce Motion. Follows the system's Accessibility setting live, and the animation tokens in `T` read it.
/// Check runs can force it with `-naviaReduceMotion YES|NO`.
@Observable final class Motion {
    static let shared = Motion()
    /// Observed by views whose look depends on it (a spinner's clock).
    private(set) var reduced = false
    /// The same flag, readable from the animation tokens, which aren't on the main actor.
    nonisolated private static let flag = Atomic<Bool>(false)
    nonisolated static var isReduced: Bool { flag.load(ordering: .relaxed) }
    @ObservationIgnored private var forced: Bool?
    @ObservationIgnored private var observer: NSObjectProtocol?

    private init() {
        if UserDefaults.standard.object(forKey: "naviaReduceMotion") != nil { forced = UserDefaults.standard.bool(forKey: "naviaReduceMotion") }
        observer = NSWorkspace.shared.notificationCenter.addObserver(forName: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification,
                                                                      object: nil, queue: .main) { _ in
            MainActor.assumeIsolated { Motion.shared.update() }
        }
        update()
    }

    /// Check runs: force reduced motion on or off, or nil to follow the system again.
    func force(_ value: Bool?) {
        forced = value
        update()
    }

    private func update() {
        let on = forced ?? NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
        Self.flag.store(on, ordering: .relaxed)
        if reduced != on { reduced = on }
    }
}
