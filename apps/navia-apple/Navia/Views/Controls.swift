import AppKit
import SwiftUI

extension View {
    /// Platform glass in a capsule, tinted with `tint`, with a thin light edge.
    func glassPill(tint: RGBA = T.glass) -> some View {
        modifier(GlassFill(shape: AnyShape(Capsule()), tint: tint))
            .overlay(Capsule().strokeBorder(T.glassEdge.color, lineWidth: 1).allowsHitTesting(false))
    }

    /// Platform glass in a rounded rectangle. `veil` is a color laid over the blur to pull it toward the window's
    /// color. `inactiveVeil` replaces it while the window is inactive, where the platform glass lightens.
    func glassPanel(radius: CGFloat, tint: RGBA, veil: RGBA? = nil, inactiveVeil: RGBA? = nil) -> some View {
        modifier(GlassFill(shape: AnyShape(RoundedRectangle(cornerRadius: radius, style: .continuous)), tint: tint, veil: veil, inactiveVeil: inactiveVeil))
    }

    /// The panel glass of the sidebar and Settings' cards. Its edge is the platform glass's own.
    func panelGlass(radius: CGFloat) -> some View {
        glassPanel(radius: radius, tint: T.panelGlass, veil: T.panelGlassVeil, inactiveVeil: T.panelGlassVeilInactive)
    }

    /// A floating panel, used by popups, the smart rules and the volume panel. `clip` keeps scrolling content inside
    /// the corners.
    func floatPanel(clip: Bool = false, radius: CGFloat = T.rFloat) -> some View { modifier(FloatPanel(clip: clip, radius: radius)) }

    func shadow(_ token: T.Shadow) -> some View {
        shadow(color: token.color.color, radius: T.px(token.blur / 2), x: T.px(token.x), y: T.px(token.y))
    }

    /// The accent ring just outside a focused field or an open select. An inset border would shift the contents.
    func focusRing(_ on: Bool, _ color: RGBA, width: CGFloat = T.focusRingWidth) -> some View {
        overlay(Capsule().strokeBorder(color.color, lineWidth: width).padding(-width).opacity(on ? 1 : 0).allowsHitTesting(false))
    }

    func invalidRing(_ on: Bool) -> some View { focusRing(on, T.invalidRing, width: T.invalidRingWidth) }

    func artShadow() -> some View { shadow(T.shadowArt) }
}

/// A one-time question inside a panel, such as the queue's Deezer card. A title, one fact beside its glyph, then
/// the answers side by side.
struct PromptCard<Answers: View>: View {
    let title: String
    let symbol: String
    let fact: String
    @ViewBuilder let answers: Answers

    var body: some View {
        VStack(alignment: .leading, spacing: T.promptCardGap) {
            Text(title).font(.navia(T.textMD, .semibold)).foregroundStyle(T.tEm.color).fixedSize(horizontal: false, vertical: true)
            HStack(alignment: .firstTextBaseline, spacing: T.promptCardGap) {
                Image(systemName: symbol).font(.system(size: T.textSM)).foregroundStyle(T.t2.color)
                Text(fact).font(.navia(T.textSM)).foregroundStyle(T.t2.color).fixedSize(horizontal: false, vertical: true)
            }
            HStack(spacing: T.dialogActionsGap) { answers }
        }
        .padding(T.promptCardPad)
        .frame(maxWidth: .infinity, alignment: .leading)
        .floatPanel(radius: T.rLG)
    }
}

private struct FloatPanel: ViewModifier {
    let clip: Bool
    /// `T.rFloat`, or `T.rLg` for a card standing on its own.
    let radius: CGFloat

    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: radius, style: .continuous)
        Group {
            if clip {
                content.glassPanel(radius: radius, tint: T.floatGlass).clipShape(shape)
            } else {
                content.glassPanel(radius: radius, tint: T.floatGlass)
            }
        }
        .overlay(shape.strokeBorder(T.glassEdge.color, lineWidth: 1).allowsHitTesting(false))
        .shadow(T.shadowFloat)
    }
}

/// Platform glass in the window. Offscreen check images can't draw glass, so they get its flat tint instead.
private struct GlassFill: ViewModifier {
    let shape: AnyShape
    let tint: RGBA
    var veil: RGBA? = nil
    var inactiveVeil: RGBA? = nil
    @Environment(\.isSnapshot) private var isSnapshot
    @Environment(\.appearsActive) private var active

    func body(content: Content) -> some View {
        let shown = active ? veil : (inactiveVeil ?? veil)
        let veiled = content.background { if let shown { shape.fill(shown.color) } }
        if isSnapshot {
            veiled.background(shape.fill(tint.color))
        } else {
            veiled.glassEffect(.regular.tint(tint.color), in: shape)
        }
    }
}

struct GlassGroup<Content: View>: View {
    @ViewBuilder var content: Content
    var body: some View {
        HStack(spacing: 0) { content }
            .padding(.horizontal, T.px(3))
            .frame(height: T.controlH)
            .glassPill()
    }
}

/// A Play pill with a filled glyph and its Shuffle partner, followed by a page's own actions in a second pill.
struct PlayActions<Extra: View>: View {
    let songs: [SongRow]
    let shell: ShellModel
    @ViewBuilder var extra: Extra

    var body: some View {
        let none = songs.isEmpty
        GlassGroup {
            TextPillButton(symbol: "play.fill", title: L("Play"), disabled: none, key: "Play") { shell.playAll(songs) }
                .help(none ? L("No songs to play") : L("Play"))
            GlassDivider()
            IconButton(symbol: "shuffle", label: none ? L("No songs to play") : L("Shuffle"), disabled: none) { shell.playAll(songs, shuffle: true) }
        }
        if Extra.self != EmptyView.self { GlassGroup { extra } }
    }
}

/// A text button inside a glass group, with a filled symbol and a label. Disabled ones are dimmed.
struct TextPillButton: View {
    /// Nil for a word alone.
    let symbol: String?
    let title: String
    var disabled = false
    /// The name `-naviaForceHover` knows it by.
    var key: String? = nil
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: T.px(9)) {
                if let symbol { Image(systemName: symbol).font(.system(size: T.px(16))) }
                Text(title).font(.navia(T.textMD, .medium))
            }
            .padding(.leading, T.px(13))
            .padding(.trailing, T.px(12))
            .frame(height: T.iconH)
            .contentShape(Capsule())
        }
        .buttonStyle(HoverFillStyle(key: key ?? title))
        .disabled(disabled)
        .opacity(disabled ? T.disabledOpacity : 1)
    }
}

/// An action button in a dialog, form or page, using the system's glass buttons. `.prominent` is tinted glass and
/// `.secondary` is plain glass, with system hover, pressed and disabled looks. `compact` is the smaller size.
/// `small` is drawn by Navia, since the system's glass buttons have no size that matches it.
struct ActionButton: View {
    /// `.primary` is a solid `fill` with `ink` text. Navia draws it when `small`, and uses prominent glass otherwise.
    enum Kind { case secondary, prominent(tint: RGBA), primary(fill: RGBA, ink: RGBA) }
    let title: String
    var symbol: String? = nil
    var kind: Kind = .secondary
    var compact = false
    var small = false
    var enabled = true
    /// Shares the row's width with its neighbors.
    var fill = false
    var busy = false
    let action: () -> Void

    var body: some View {
        if small {
            Button(action: action) {
                HStack(spacing: T.buttonGap) {
                    if let symbol { Image(systemName: symbol).font(.system(size: T.chipGlyph)) }
                    Text(title).lineLimit(1)
                }
                    .font(.navia(T.textXS, isPrimary ? .semibold : .medium))
                    .fixedSize()
                    .padding(.horizontal, T.buttonSmallPad)
                    .frame(maxWidth: fill ? .infinity : nil)
                    .frame(height: T.buttonSmallH)
                    .modifier(SmallButtonFill(kind: kind, enabled: enabled))
                    .contentShape(Capsule())
            }
            .buttonStyle(HoverFillStyle(key: title))
            .disabled(!enabled)
            // A disabled primary keeps a faint fill and a legible label instead of fading whole.
            .opacity(enabled || isPrimary ? 1 : T.disabledOpacity)
        } else {
            system
        }
    }

    private var isPrimary: Bool { if case .primary = kind { return true }; return false }

    @ViewBuilder private var system: some View {
        let button = Button(action: action) {
            HStack(spacing: T.buttonGap) {
                if busy { Spinner(color: T.tEm).frame(width: T.px(16), height: T.px(16)) }
                if let symbol { Image(systemName: symbol) }
                Text(title).lineLimit(1)
            }
            .font(.navia(compact ? T.textSM : T.textMD, .medium))
            .fixedSize()
            .frame(maxWidth: fill ? .infinity : nil)
        }
        .controlSize(compact ? .large : .extraLarge)
        .disabled(!enabled)
        switch kind {
        case .secondary: button.buttonStyle(.glass)
        case .prominent(let tint), .primary(let tint, _): button.buttonStyle(.glassProminent).tint(tint.color)
        }
    }
}

/// A small button's fill. A disabled primary keeps `disabledOpacity` of its fill with the `t2` label.
private struct SmallButtonFill: ViewModifier {
    let kind: ActionButton.Kind
    let enabled: Bool
    func body(content: Content) -> some View {
        switch kind {
        case .secondary: content.glassPill()
        case .prominent(let tint): content.glassPill(tint: tint)
        case .primary(let fill, let ink):
            content
                .foregroundStyle((enabled ? ink : T.t2).color)
                .background(Capsule().fill(fill.opacity(enabled ? 1 : T.disabledOpacity).color))
                .overlay(Capsule().strokeBorder(T.glassEdge.color, lineWidth: 1).allowsHitTesting(false))
        }
    }
}

struct GlassDivider: View {
    var body: some View {
        Rectangle().fill(T.glassDivider.color).frame(width: 1, height: T.px(20)).padding(.horizontal, T.px(3))
    }
}

struct IconButton: View {
    let symbol: String
    let label: String
    var size: CGFloat = T.px(17)
    var weight: Font.Weight = .regular
    var active = false
    var inGlass = false
    var width: CGFloat = T.iconW
    var height: CGFloat = T.iconH
    var disabled = false
    /// The tint for shuffle and repeat when on.
    var activeTint: Color? = nil
    /// A drawn glyph in place of the symbol, such as the Download switch's progress ring.
    var glyph: AnyView? = nil
    var help: String? = nil
    /// Window controls over a cover, on a dark fill that darkens on hover.
    var onArt = false
    var action: () -> Void = {}
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            Group {
                if let glyph {
                    glyph
                } else {
                    Image(systemName: symbol)
                        .font(.system(size: size, weight: weight))
                        .symbolVariant(active && weight != .semibold && weight != .bold ? .fill : .none)
                        .foregroundStyle(active && activeTint != nil ? AnyShapeStyle(activeTint!) : AnyShapeStyle(.foreground))
                }
            }
                .frame(width: width, height: height)
                .background(Capsule().fill(active && inGlass ? T.glassOn.color : .clear))
                .background(Capsule().fill(onArt ? (hovering ? T.windowButtonOnArtHover : T.windowButtonOnArt).color : .clear))
                .contentShape(Capsule())
        }
        .buttonStyle(HoverBrightenStyle(selected: active && inGlass, key: label))
        .opacity(disabled ? 0.3 : (active || hovering || HoverDebug.forced(label) ? 1 : 0.88))
        .disabled(disabled)
        .onHover { hovering = $0 }
        .help(help ?? label)
        .accessibilityLabel(label)
    }

}

/// Hands wheel events over a hovered slider to that slider. It uses a local monitor because SwiftUI has no wheel
/// event on the Mac and an AppKit overlay would take the slider's drags.
@MainActor final class WheelRouter {
    static let shared = WheelRouter()
    private var handler: ((Int) -> Void)?
    private var monitor: Any?
    private var pixels: CGFloat = 0

    func enter(_ handler: @escaping (Int) -> Void) {
        self.handler = handler
        pixels = 0
        guard monitor == nil else { return }
        monitor = NSEvent.addLocalMonitorForEvents(matching: .scrollWheel) { [weak self] event in
            guard let self, let handler = self.handler else { return event }
            for step in self.steps(event) { handler(step) }
            return nil
        }
    }

    func leave() { handler = nil; pixels = 0 }

    /// One step per wheel notch. Trackpad moves add up first, and momentum after the fingers lift is ignored.
    /// Scrolling up is louder.
    private func steps(_ event: NSEvent) -> [Int] {
        guard event.momentumPhase.isEmpty, event.scrollingDeltaY != 0 else { return [] }
        guard event.hasPreciseScrollingDeltas else { return [event.scrollingDeltaY > 0 ? 1 : -1] }
        pixels += event.scrollingDeltaY
        let threshold: CGFloat = 16
        guard abs(pixels) >= threshold else { return [] }
        let count = Int(abs(pixels) / threshold)
        pixels = pixels.truncatingRemainder(dividingBy: threshold)
        return Array(repeating: event.scrollingDeltaY > 0 ? 1 : -1, count: count)
    }
}

/// A thin track for seeking and volume. The thumb fades in on hover and stays while dragging.
struct TrackSlider: View {
    let value: Double
    let total: Double
    var rest: RGBA = T.barTrack
    /// The mini player's hairline track, with a smaller thumb.
    var thin = false
    /// Text in a small chip over the playhead while hovered, kept `T.scrubChipEdge` from the ends.
    var chip: String? = nil
    /// A wheel step while hovered, 1 up and -1 down. Nil leaves the wheel to whatever is behind the slider.
    var onWheel: ((Int) -> Void)? = nil
    let onChange: (Double) -> Void
    @State private var hovering = false
    @State private var dragging = false
    @State private var chipHeight: CGFloat = 0

    var body: some View {
        GeometryReader { geo in
            let fraction = total > 0 ? min(1, max(0, value / total)) : 0
            let line = thin ? T.scrubThin : 4
            let knob = thin ? T.scrubThinKnob : T.scrubKnob
            let shown = hovering || dragging
            ZStack(alignment: .leading) {
                Capsule().fill(rest.color).frame(height: line)
                Capsule().fill(.foreground).frame(width: geo.size.width * fraction, height: line)
                Circle().fill(T.knob.color).frame(width: knob, height: knob)
                    .shadow(T.shadowKnob)
                    .scaleEffect(shown ? 1 : T.scrubKnobRestScale)
                    .opacity(shown ? 1 : 0)
                    .animation(T.hover(shown), value: shown)
                    .offset(x: geo.size.width * fraction - knob / 2)
                    .allowsHitTesting(false)
            }
            .frame(maxHeight: .infinity)
            .overlay(alignment: .topLeading) {
                if let chip {
                    let x = min(max(geo.size.width * fraction, T.scrubChipEdge), max(T.scrubChipEdge, geo.size.width - T.scrubChipEdge))
                    Text(chip)
                        .font(.navia(T.textXS).monospacedDigit())
                        .foregroundStyle(T.t1.color)
                        .lineLimit(1)
                        .padding(.horizontal, T.px(8))
                        .padding(.vertical, T.px(3))
                        .glassPill(tint: T.floatGlass)
                        .fixedSize()
                        .alignmentGuide(.leading) { $0[HorizontalAlignment.center] - x }
                        // Sits 2 px above the bar. `.alignmentGuide(.top)` is ignored here, so the measured height places it.
                        .onGeometryChange(for: CGFloat.self, of: { $0.size.height }) { chipHeight = $0 }
                        .offset(y: -(chipHeight + T.px(2)))
                        .opacity(hovering || HoverDebug.forced("ScrubChip") ? 1 : 0)
                        .animation(T.hover(hovering), value: hovering)
                        .allowsHitTesting(false)
                }
            }
            .contentShape(Rectangle())
            .gesture(DragGesture(minimumDistance: 0).onChanged { drag in
                dragging = true
                onChange(min(1, max(0, drag.location.x / max(1, geo.size.width))) * total)
            }.onEnded { _ in dragging = false })
        }
        .frame(height: T.px(14))
        .onHover { over in
            hovering = over
            guard let onWheel else { return }
            if over { WheelRouter.shared.enter(onWheel) } else { WheelRouter.shared.leave() }
        }
        .onDisappear { if hovering, onWheel != nil { WheelRouter.shared.leave() } }
    }
}

/// The system's switch, tinted with the accent. `label` is for VoiceOver.
struct ToggleSwitch: View {
    @Binding var isOn: Bool
    let accent: RGBA
    let label: String

    var body: some View {
        Toggle(label, isOn: $isOn)
            .toggleStyle(.switch)
            .labelsHidden()
            .tint(accent.color)
    }
}


/// The accent count pill on a drag card or a lifted row.
struct CountBadge: View {
    let text: String
    let accent: RGBA
    let onAccent: RGBA
    var body: some View {
        Text(text)
            .font(.navia(T.px(12), .semibold))
            .foregroundStyle(onAccent.color)
            .padding(.horizontal, T.px(8))
            .frame(height: T.px(20))
            .background(Capsule().fill(accent.color))
            .shadow(T.shadowBadge)
            .fixedSize()
    }
}

/// Lays items out left to right, wrapping when the width runs out, each line centered vertically. An item wider
/// than the line gets the line's width, so long text wraps.
struct FlowChips: Layout {
    let spacing: CGFloat
    var lineSpacing: CGFloat = 6

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        arrange(subviews, width: proposal.width ?? .infinity).size
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let layout = arrange(subviews, width: bounds.width)
        for (index, frame) in layout.frames.enumerated() {
            subviews[index].place(at: CGPoint(x: bounds.minX + frame.minX, y: bounds.minY + frame.minY), proposal: ProposedViewSize(frame.size))
        }
    }

    private func arrange(_ subviews: Subviews, width: CGFloat) -> (frames: [CGRect], size: CGSize) {
        var lines: [[Int]] = [[]]
        var sizes: [CGSize] = []
        var x: CGFloat = 0
        for (index, sub) in subviews.enumerated() {
            var size = sub.sizeThatFits(.unspecified)
            if size.width > width { size = sub.sizeThatFits(ProposedViewSize(width: width, height: nil)) }
            if x > 0 && x + size.width > width { lines.append([]); x = 0 }
            lines[lines.count - 1].append(index)
            sizes.append(size)
            x += size.width + spacing
        }
        var frames = [CGRect](repeating: .zero, count: sizes.count)
        var y: CGFloat = 0, right: CGFloat = 0
        for (n, line) in lines.enumerated() where !line.isEmpty {
            let height = line.map { sizes[$0].height }.max() ?? 0
            var lx: CGFloat = 0
            for index in line {
                frames[index] = CGRect(x: lx, y: y + (height - sizes[index].height) / 2, width: sizes[index].width, height: sizes[index].height)
                lx += sizes[index].width + spacing
            }
            right = max(right, lx - spacing)
            y += height + (n < lines.count - 1 ? lineSpacing : 0)
        }
        return (frames, CGSize(width: right, height: y))
    }
}

/// Decides which chips of a one-line row show. The line never wraps or overflows. Kept chips show first, then the
/// others in order until one doesn't fit, and the rest fold into More. The end group (`endW`) sits at the right
/// end, and moves into More (`endInMore`) when the kept chips leave it no room. `pin` always shows, and `rest`
/// keeps More visible even when every chip fits.
enum ChipFold {
    static func fold(widths: [CGFloat], keep: [Bool], pin: Int = -1, moreW: CGFloat, endW: CGFloat, gap: CGFloat, room: CGFloat, rest: Bool) -> (shown: [Bool], more: Bool, endInMore: Bool) {
        let endRoom = endW > 0 ? gap + endW : 0
        let all = widths.reduce(0) { $0 + $1 + gap } - (widths.isEmpty ? 0 : gap)
        if !rest && all + endRoom <= room { return (widths.map { _ in true }, false, false) }
        let must = widths.indices.map { keep[$0] || $0 == pin }
        let kept = zip(widths, must).reduce(CGFloat(0)) { $0 + ($1.1 ? $1.0 + gap : 0) }
        let budget = room - endRoom - moreW
        if kept <= budget {
            var used = kept, folding = false
            let shown = zip(widths, must).map { w, m -> Bool in
                if m { return true }
                if !folding && used + w + gap <= budget { used += w + gap; return true }
                folding = true
                return false
            }
            return (shown, true, false)
        }
        var used = pin >= 0 && pin < widths.count ? widths[pin] + gap : 0, folding = false
        let shown = widths.indices.map { i -> Bool in
            if i == pin { return true }
            if keep[i] && !folding && used + widths[i] + gap <= room - moreW { used += widths[i] + gap; return true }
            if keep[i] { folding = true }
            return false
        }
        return (shown, true, endW > 0)
    }
}

struct ChipValue: View {
    let text: String
    var body: some View {
        Text(text).lineLimit(1).truncationMode(.tail).frame(maxWidth: T.chipValueMaxW, alignment: .leading).fixedSize(horizontal: false, vertical: true)
    }
}

/// A filter chip, `T.chipH` tall, glass at rest and `T.chipOn` when on. `menu` adds a caret for a popover, and
/// `remove` adds an X button that shares the chip's shape.
struct FilterChip<Label: View>: View {
    var on = false
    var menu = false
    var remove: (() -> Void)? = nil
    var removeLabel = ""
    let action: () -> Void
    @ViewBuilder var label: Label
    @State private var hovering = false

    var body: some View {
        HStack(spacing: 0) {
            Button(action: action) {
                HStack(spacing: T.chipGap) {
                    label
                    if menu && remove == nil { Image(systemName: "chevron.down").font(.system(size: T.chipGlyph * 0.8, weight: .bold)).opacity(0.7) }
                }
                .padding(.leading, T.chipPad)
                .padding(.trailing, remove == nil ? T.chipPad : T.chipGap)
                .frame(height: T.chipH)
                .contentShape(Capsule())
            }
            .buttonStyle(.plain)
            if let remove {
                Button(action: remove) {
                    Image(systemName: "xmark").font(.system(size: T.chipGlyph * 0.8, weight: .bold))
                        .padding(.leading, T.chipGap)
                        .padding(.trailing, T.chipPad - T.chipGap)
                        .frame(height: T.chipH)
                        .contentShape(Capsule())
                }
                .buttonStyle(ChipRemoveStyle())
                .help(removeLabel)
                .accessibilityLabel(removeLabel)
            }
        }
        .font(.navia(T.textSM, .medium))
        .lineLimit(1)
        .fixedSize()
        .foregroundStyle((on ? T.onChip : T.t1).color)
        .background {
            if on {
                Capsule().fill(T.chipOn.color)
            } else {
                Capsule().fill((hovering ? T.glassHover : T.glass).color)
                    .overlay(Capsule().strokeBorder(T.glassEdge.color, lineWidth: 1))
                    .animation(T.hover(hovering), value: hovering)
            }
        }
        .onHover { hovering = $0 }
    }
}

private struct ChipRemoveStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View { ChipRemoveBody(label: configuration.label) }
}

private struct ChipRemoveBody<Label: View>: View {
    let label: Label
    @State private var hovering = false
    var body: some View { label.opacity(hovering ? 1 : 0.7).onHover { hovering = $0 } }
}

/// A number field, such as Year or BPM. It shows the focus ring while typing and the invalid ring when its value
/// is the problem.
struct NumberField: View {
    let placeholder: String
    @Binding var text: String
    let bad: Bool
    let accent: RGBA
    @FocusState private var focused: Bool

    var body: some View {
        TextField("", text: $text, prompt: Text(placeholder).foregroundStyle(T.t3.color))
            .textFieldStyle(.plain)
            .font(.navia(T.textSM).monospacedDigit())
            .focused($focused)
            .padding(.horizontal, T.chipPad)
            .frame(width: T.numberFieldW, height: T.chipH)
            .background(Capsule().fill(T.row.color))
            .focusRing(focused, accent)
            .invalidRing(bad && !focused)
            .accessibilityLabel(placeholder)
    }
}

/// A floating panel anchored under its trigger, or above when there's more room, kept inside the window.
/// A click outside closes it. `wanted` is the content's natural height, capped at `T.popMaxH`.
struct AnchoredPopover<Content: View>: View {
    let anchor: CGRect
    let width: CGFloat
    let wanted: CGFloat
    let windowSize: CGSize
    /// The least it shrinks to when the window is short.
    var minHeight: CGFloat = T.px(120)
    let close: () -> Void
    @ViewBuilder var content: Content
    @Environment(\.isSnapshot) private var isSnapshot

    var body: some View {
        let edge = T.px(10), gap = T.px(6), w = min(width, windowSize.width - edge * 2)
        let capped = min(wanted, T.popMaxH)
        let below = windowSize.height - anchor.maxY - gap - edge
        let above = anchor.minY - gap - edge
        let down = below >= capped || below >= above
        let height = max(minHeight, min(capped, down ? below : above))
        let x = max(edge, min(anchor.minX, windowSize.width - w - edge))
        let y = down ? anchor.maxY + gap : max(edge, anchor.minY - gap - height)
        return ZStack(alignment: .topLeading) {
            Color.clear
                .contentShape(Rectangle())
                .onTapGesture { close() }
                .accessibilityHidden(true)
            content
                .frame(width: w, height: isSnapshot ? T.popMaxH : wanted == 0 ? nil : height, alignment: .top)
                .floatPanel(clip: true)
                .offset(x: x, y: y)
                .opacity(wanted == 0 && !isSnapshot ? 0 : 1)
        }
        .frame(width: windowSize.width, height: windowSize.height, alignment: .topLeading)
    }
}
