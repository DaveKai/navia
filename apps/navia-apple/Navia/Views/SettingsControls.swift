import AppKit
import SwiftUI

/// A section title and its rounded card of rows.
struct SettingsSection<Content: View>: View {
    let title: String
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: T.px(10)) {
            Text(title).font(.navia(T.textSection, .semibold)).tracking(-0.012 * T.textSection)
            SettingsCard { content }
        }
    }
}

/// A group of rows on the panel glass, so the window's translucency shows through like in the sidebar.
struct SettingsCard<Content: View>: View {
    @ViewBuilder let content: Content
    var body: some View {
        VStack(spacing: 0) { content }
            .clipShape(RoundedRectangle(cornerRadius: T.px(14), style: .continuous))
            .panelGlass(radius: T.px(14))
    }
}

/// One row of a settings card, with a label, an optional hint and its control.
/// `dim` marks a row whose control has no backend yet, and the hint says why.
struct SettingsRow<Control: View>: View {
    let label: String
    var hint: String?
    var dim = false
    /// The last row of a card has no separator.
    var last = false
    @ViewBuilder let control: Control
    @Environment(\.pageContext) private var ctx

    var body: some View {
        let wrap = ctx.cssWidth <= 620
        Group {
            if wrap {
                VStack(alignment: .leading, spacing: T.px(8)) { text; control }
            } else {
                HStack(spacing: T.px(24)) { text; control }
            }
        }
        .padding(.vertical, T.px(10))
        .padding(.horizontal, T.px(16))
        .frame(maxWidth: .infinity, minHeight: T.px(60), alignment: .leading)
        .overlay(alignment: .bottom) { if !last { Rectangle().fill(T.line.color).frame(height: 1) } }
    }

    private var text: some View {
        VStack(alignment: .leading, spacing: T.px(1)) {
            Text(label).font(.navia(T.textMD, .medium)).foregroundStyle((dim ? T.t2 : T.t1).color)
            if let hint {
                Text(hint).font(.navia(T.textSM)).foregroundStyle(T.t2.color).fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// A row that opens another page, with a caret.
struct SettingsLinkRow: View {
    let label: String
    let hint: String
    var last = false
    /// Shown before the chevron, like a size.
    var value: String? = nil
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: T.px(24)) {
                VStack(alignment: .leading, spacing: T.px(1)) {
                    Text(label).font(.navia(T.textMD, .medium)).foregroundStyle(T.t1.color)
                    Text(hint).font(.navia(T.textSM)).foregroundStyle(T.t2.color)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                if let value { Text(value).font(.navia(T.textMD)).foregroundStyle(T.t2.color).fixedSize() }
                Image(systemName: "chevron.right").font(.system(size: T.px(14), weight: .bold)).foregroundStyle(T.t3.color)
            }
            .padding(.vertical, T.px(10))
            .padding(.horizontal, T.px(16))
            .frame(maxWidth: .infinity, minHeight: T.px(60))
            .background(hovering ? T.hover.color : .clear)
            .overlay(alignment: .bottom) { if !last { Rectangle().fill(T.line.color).frame(height: 1) } }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
    }
}

/// A row control with no backend yet, dimmed, with the current value and a caret.
struct DisabledPill: View {
    let value: String
    var width: CGFloat? = T.px(240)

    var body: some View {
        HStack(spacing: T.px(6)) {
            Text(value).font(.navia(T.textMD)).lineLimit(1).frame(maxWidth: .infinity, alignment: .leading)
            Image(systemName: "chevron.up.chevron.down").font(.system(size: T.px(11), weight: .medium)).foregroundStyle(T.t2.color)
        }
        .padding(.leading, T.px(16))
        .padding(.trailing, T.px(12))
        .frame(width: width, height: T.controlH)
        .glassPill()
        .opacity(T.disabledOpacity)
    }
}

/// The native segmented control, at Settings' size or the larger size used in page toolbars.
struct SegmentedChoice<Value: Hashable>: View {
    struct Option { let value: Value; let label: String; var disabledReason: String? = nil }

    /// `settings` for Settings' rows, `glass` for page toolbars.
    enum Look { case settings, glass }
    let look: Look
    let value: Value
    let options: [Option]
    let accent: RGBA
    let label: String
    let onChange: (Value) -> Void

    var body: some View {
        // The native segmented control, tinted with the accent. A disabled option is a disabled segment with its reason
        // as the tooltip.
        NativeSegments(labels: options.map(\.label), selected: options.firstIndex { $0.value == value },
                       disabled: options.map { $0.disabledReason }, accent: accent, large: look == .glass) { index in
            let option = options[index]
            if option.disabledReason == nil, option.value != value { onChange(option.value) }
        }
        .fixedSize()
        .accessibilityLabel(label)
    }
}

/// `NSSegmentedControl`, which can disable and explain single segments and takes the accent as its selected color.
private struct NativeSegments: NSViewRepresentable {
    let labels: [String]
    let selected: Int?
    let disabled: [String?]
    let accent: RGBA
    let large: Bool
    let onSelect: (Int) -> Void

    func makeNSView(context: Context) -> NSSegmentedControl {
        let control = NSSegmentedControl(labels: labels, trackingMode: .selectOne, target: context.coordinator, action: #selector(Coordinator.changed(_:)))
        control.segmentStyle = .automatic
        return control
    }

    func updateNSView(_ control: NSSegmentedControl, context: Context) {
        context.coordinator.onSelect = onSelect
        if control.segmentCount != labels.count { control.segmentCount = labels.count }
        for (i, text) in labels.enumerated() {
            control.setLabel(text, forSegment: i)
            control.setEnabled(disabled[i] == nil, forSegment: i)
            control.setToolTip(disabled[i], forSegment: i)
            control.setWidth(0, forSegment: i)
        }
        control.selectedSegment = selected ?? -1
        control.selectedSegmentBezelColor = NSColor(accent.color)
        control.controlSize = large ? .large : .regular
        control.font = NSFont.systemFont(ofSize: large ? T.textMD : T.textSM, weight: .medium)
    }

    func makeCoordinator() -> Coordinator { Coordinator(onSelect: onSelect) }

    final class Coordinator: NSObject {
        var onSelect: (Int) -> Void
        init(onSelect: @escaping (Int) -> Void) { self.onSelect = onSelect }
        @objc func changed(_ sender: NSSegmentedControl) { onSelect(sender.selectedSegment) }
    }
}

/// The range control for Settings and the Equalizer.
struct PlainSlider: View {
    enum Look: Equatable {
        /// The native slider, tinted with the accent.
        case native(accent: RGBA)
        /// The fill grows from `origin` in the accent color.
        case fill(origin: Double?, accent: RGBA)
        /// The accent picker's rainbow.
        case hue
    }

    let label: String
    let value: Double
    let min: Double
    let max: Double
    var step: Double = 1
    var look: Look
    var width: CGFloat? = T.px(200)
    var disabled = false
    var valueText: String? = nil
    let onChange: (Double) -> Void
    var onEnd: () -> Void = {}

    var body: some View {
        if case .native(let accent) = look {
            Slider(value: Binding(get: { value }, set: { next in
                let stepped = step > 0 ? (next / step).rounded() * step : next
                onChange(Swift.min(max, Swift.max(min, stepped)))
            }), in: min...max, onEditingChanged: { editing in if !editing { onEnd() } })
            .labelsHidden()
            .tint(accent.color)
            .disabled(disabled)
            .frame(width: width)
            .accessibilityLabel(label)
            .accessibilityValue(valueText ?? String(format: "%g", value))
        } else {
            drawn
        }
    }

    /// The Equalizer's fill and the accent picker's rainbow, drawn by Navia.
    private var drawn: some View {
        GeometryReader { geo in
            let thumb = T.px(14)
            let usable = Swift.max(1, geo.size.width - thumb)
            let fraction = Swift.min(1, Swift.max(0, (value - min) / (max - min)))
            ZStack(alignment: .leading) {
                track(usable: usable, thumb: thumb)
                Circle().fill(look == .hue ? T.knob.color : T.t1.color)
                    .frame(width: thumb, height: thumb)
                    .shadow(look == .hue ? T.shadowKnobHue : T.shadowKnob)
                    .offset(x: fraction * usable)
            }
            .frame(maxHeight: .infinity)
            .contentShape(Rectangle())
            .gesture(DragGesture(minimumDistance: 0)
                .onChanged { drag in
                    guard !disabled else { return }
                    let f = Swift.min(1, Swift.max(0, (drag.location.x - thumb / 2) / usable))
                    var next = min + f * (max - min)
                    if step > 0 { next = (next / step).rounded() * step }
                    onChange(Swift.min(max, Swift.max(min, next)))
                }
                .onEnded { _ in onEnd() })
        }
        .frame(width: width, height: T.px(16))
        .opacity(disabled ? T.disabledOpacity : 1)
        .accessibilityElement()
        .accessibilityLabel(label)
        .accessibilityValue(valueText ?? String(format: "%g", value))
        .accessibilityAdjustableAction { direction in
            guard !disabled else { return }
            let delta = step > 0 ? step : (max - min) / 20
            onChange(Swift.min(max, Swift.max(min, value + (direction == .increment ? delta : -delta))))
            onEnd()
        }
    }

    @ViewBuilder private func track(usable: CGFloat, thumb: CGFloat) -> some View {
        switch look {
        case .hue:
            Capsule()
                .fill(LinearGradient(colors: ["ff6666", "ffcc66", "99ff66", "66ffcc", "66ccff", "9966ff", "ff66cc", "ff6666"].compactMap { RGBA(hex: $0)?.color }, startPoint: .leading, endPoint: .trailing))
                .frame(height: T.px(6))
        case .native:
            EmptyView()
        case .fill(let origin, let accent):
            let pct: (Double) -> CGFloat = { v in thumb / 2 + CGFloat(Swift.min(1, Swift.max(0, (v - min) / (max - min)))) * usable }
            ZStack(alignment: .leading) {
                Capsule().fill(T.panel2.color).frame(height: T.px(4))
                let (from, to) = (pct(Swift.min(value, origin ?? min)), pct(Swift.max(value, origin ?? min)))
                Capsule().fill(accent.color).frame(width: Swift.max(0, to - from), height: T.px(4)).offset(x: from)
            }
        }
    }
}

/// A plain text field in a pill, for the accent hex, preset names and rule values.
struct PillTextField: View {
    let placeholder: String
    @Binding var text: String
    let accent: RGBA
    var width: CGFloat? = nil
    var height: CGFloat = T.controlH
    /// A red ring for a value that isn't valid.
    var invalid = false
    var font: Font = .navia(T.textMD)
    var onSubmit: () -> Void = {}
    var onEscape: () -> Void = {}
    /// Called when the field loses focus, for a value that applies once it's entered.
    var onBlur: () -> Void = {}
    var filled = false
    /// Takes focus when it appears, after AppKit's own launch focus has settled.
    var autoFocus = false
    /// Room kept at the right end for an action laid over it.
    var trailingRoom: CGFloat = 0
    @FocusState private var focused: Bool

    var body: some View {
        TextField("", text: $text, prompt: Text(placeholder).foregroundStyle(T.t3.color))
            .textFieldStyle(.plain)
            .font(font)
            .focused($focused)
            .onSubmit(onSubmit)
            .onKeyPress(.escape) { onEscape(); return .ignored }
            .onChange(of: focused) { _, now in if !now { onBlur() } }
            .padding(.horizontal, filled ? T.px(12) : T.fieldPad)
            .padding(.trailing, trailingRoom)
            .frame(width: width, height: height)
            .frame(maxWidth: width == nil ? .infinity : nil)
            .modifier(FieldBackground(filled: filled))
            .focusRing(focused || invalid, ring, width: invalid ? T.invalidRingWidth : T.focusRingWidth)
            .task {
                guard autoFocus else { return }
                try? await Task.sleep(for: .milliseconds(150))
                focused = true
            }
    }

    /// The invalid ring wins over focus.
    private var ring: RGBA { invalid ? T.invalidRing : accent }
}

private struct FieldBackground: ViewModifier {
    let filled: Bool
    func body(content: Content) -> some View {
        if filled { content.background(Capsule().fill(T.panel2.color)) } else { content.glassPill() }
    }
}

/// The default accent and six others, matching GPUI's `ACCENT_PRESETS`.
let accentPresets = ["5a93f5", "50ffff", "8b7cf6", "e879a6", "f2685c", "f5a33d", "4cc38a"]

/// Presets, a hue slider and a hex field.
struct AccentPicker: View {
    let theme: Theme
    @Environment(\.pageContext) private var ctx
    @State private var text = ""

    var body: some View {
        let wrap = ctx.cssWidth <= 760
        let layout = wrap ? AnyLayout(FlowLayoutWrap(spacing: T.px(12))) : AnyLayout(HStackLayout(spacing: T.px(12)))
        layout {
            HStack(spacing: T.px(7)) {
                ForEach(accentPresets, id: \.self) { swatch($0) }
            }
            PlainSlider(label: L("Accent hue"), value: theme.hue, min: 0, max: 359, look: .hue, width: T.px(96),
                        onChange: { theme.setAccent(hue: $0) })
            PillTextField(placeholder: "", text: Binding(get: { text }, set: { typed($0) }), accent: theme.accent, width: T.px(84), height: T.px(34),
                          invalid: !isHex(text), font: .navia(T.textSM), filled: true)
        }
        .onAppear { text = theme.hex }
        .onChange(of: theme.hex) { _, new in if !isHex(text) || text.lowercased() != new { text = new } }
    }

    private func isHex(_ value: String) -> Bool { value.hasPrefix("#") && value.count == 7 && RGBA(hex: value) != nil }

    private func typed(_ value: String) {
        text = String(value.prefix(7))
        if isHex(text) { theme.setAccent(hex: text.lowercased()) }
    }

    private func swatch(_ hex: String) -> some View {
        let on = theme.hex == "#\(hex)"
        return Button { theme.setAccent(hex: "#\(hex)") } label: {
            ZStack {
                Circle().fill(RGBA(hex: hex)!.color)
                Circle().strokeBorder(T.glassEdge.color, lineWidth: 1)
                if on { Image(systemName: "checkmark").font(.system(size: T.px(12), weight: .bold)).foregroundStyle(theme.onAccent.color) }
            }
            .frame(width: T.px(26), height: T.px(26))
            .overlay(Circle().strokeBorder(T.t1.color, lineWidth: on ? 2 : 0).padding(on ? T.px(-4) : 0))
            .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(L("Use #{hex}", ["hex": hex]))
    }
}

/// Wraps children to new lines, for the accent picker on narrow windows.
struct FlowLayoutWrap: Layout {
    var spacing: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        arrange(subviews, width: proposal.width ?? .infinity).size
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        for (index, origin) in arrange(subviews, width: bounds.width).origins.enumerated() {
            subviews[index].place(at: CGPoint(x: bounds.minX + origin.x, y: bounds.minY + origin.y), proposal: .unspecified)
        }
    }

    private func arrange(_ subviews: Subviews, width: CGFloat) -> (origins: [CGPoint], size: CGSize) {
        var x: CGFloat = 0, y: CGFloat = 0, rowH: CGFloat = 0, right: CGFloat = 0
        var origins: [CGPoint] = []
        for sub in subviews {
            let size = sub.sizeThatFits(.unspecified)
            if x > 0 && x + size.width > width { x = 0; y += rowH + spacing; rowH = 0 }
            origins.append(CGPoint(x: x, y: y))
            x += size.width + spacing
            right = max(right, x - spacing)
            rowH = max(rowH, size.height)
        }
        return (origins, CGSize(width: right, height: y + rowH))
    }
}
