import SwiftUI

/// A soft capsule hover fill on its own, for controls that aren't a SwiftUI `Button`.
struct HoverFillBackground: View {
    let on: Bool
    var pressed = false

    var body: some View {
        Capsule().fill((pressed ? T.press : T.hover).color)
        .opacity(on ? 1 : 0)
        .animation(T.hover(on), value: on)
        .allowsHitTesting(false)
    }
}

/// The fill style for text buttons. Disabled buttons get no hover. `key` names the control for the DEBUG
/// `-naviaForceHover` flag.
struct HoverFillStyle: ButtonStyle {
    var key: String? = nil

    func makeBody(configuration: Configuration) -> some View {
        HoverFillBody(label: configuration.label, pressed: configuration.isPressed, style: self)
    }
}

/// Small controls: no fill, the ink brightens to `T.hoverInk` on hover and the control dims while pressed.
/// `selected` controls keep their look.
struct HoverBrightenStyle: ButtonStyle {
    var selected = false
    var key: String? = nil

    func makeBody(configuration: Configuration) -> some View {
        HoverBrightenBody(label: configuration.label, pressed: configuration.isPressed, selected: selected, key: key)
    }
}

/// The same for controls that aren't a SwiftUI `Button`.
struct HoverBrighten: ViewModifier {
    let on: Bool
    var pressed = false
    @Environment(\.hoverInk) private var hoverInk

    func body(content: Content) -> some View {
        content
            .foregroundStyle(on ? AnyShapeStyle(hoverInk ?? T.hoverInk.color) : AnyShapeStyle(.foreground))
            .opacity(on && pressed ? T.pressDim : 1)
            .animation(T.hover(on), value: on)
            .animation(T.press(pressed), value: pressed)
    }
}

private struct HoverBrightenBody<Label: View>: View {
    let label: Label
    let pressed: Bool
    let selected: Bool
    let key: String?
    @State private var hovering = false
    @Environment(\.isEnabled) private var isEnabled

    var body: some View {
        let on = isEnabled && !selected && (hovering || HoverDebug.forced(key))
        label
            .modifier(HoverBrighten(on: on, pressed: pressed))
            .onHover { hovering = $0 }
    }
}

/// Play and skip: a little larger on hover, a little smaller while pressed, no fill.
struct HoverGrowStyle: ButtonStyle {
    var key: String? = nil

    func makeBody(configuration: Configuration) -> some View {
        HoverGrowBody(label: configuration.label, pressed: configuration.isPressed, key: key)
    }
}

private struct HoverGrowBody<Label: View>: View {
    let label: Label
    let pressed: Bool
    let key: String?
    @State private var hovering = false
    @Environment(\.isEnabled) private var isEnabled

    var body: some View {
        let on = isEnabled && (hovering || HoverDebug.forced(key))
        #if DEBUG
        let pressed = pressed || key.map { HoverLive.shared.pressed.contains($0) } == true
        #endif
        let scale = isEnabled && pressed ? T.pressShrink : on ? T.hoverGrow : 1
        label
            .scaleEffect(scale)
            #if DEBUG
            .modifier(GrowProbe(scale: scale, key: key))
            #endif
            .animation(T.hover(on), value: on)
            .animation(T.press(pressed), value: pressed)
            .onHover { hovering = $0 }
    }
}

extension EnvironmentValues {
    /// The hover ink where it isn't `T.hoverInk`, such as the tinted player's `playerFg`.
    @Entry var hoverInk: Color? = nil
}

struct HoverFillBody<Label: View>: View {
    let label: Label
    let pressed: Bool
    let style: HoverFillStyle
    @State private var hovering = false
    @Environment(\.isEnabled) private var isEnabled

    var body: some View {
        let on = isEnabled && (hovering || HoverDebug.forced(style.key))
        label
            .background { HoverFillBackground(on: on, pressed: pressed && on) }
            .onHover { hovering = $0 }
    }
}

enum HoverDebug {
    /// DEBUG only: `-naviaForceHover "Back,Shuffle"` (or `all`) shows those controls hovered. Synthetic mouse moves
    /// don't reach tracking areas, so this is how checks see hover.
    static func forced(_ key: String?) -> Bool {
        #if DEBUG
        if let key, MainActor.assumeIsolated({ HoverLive.shared.hovered.contains(key) }) { return true }
        guard let keys = forcedKeys else { return false }
        return keys.contains("all") || key.map(keys.contains) == true
        #else
        return false
        #endif
    }

    #if DEBUG
    private static let forcedKeys: Set<String>? = UserDefaults.standard.string(forKey: "naviaForceHover").map {
        Set($0.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) })
    }
    #endif
}

#if DEBUG
/// CoreCheck's live hover and press, and the frames of the grow animation they start, recorded by `GrowProbe`.
@MainActor @Observable final class HoverLive {
    static let shared = HoverLive()
    var hovered: Set<String> = []
    var pressed: Set<String> = []
    @ObservationIgnored nonisolated(unsafe) static var samples: [(time: Double, key: String, scale: Double)] = []

    /// `-naviaHoverRun KEY -naviaHoverOut FILE`: hovers KEY, presses it, lets go and leaves, then writes each phase's
    /// frames as JSON ([ms from the phase's start, scale]) and quits.
    static func runIfRequested() {
        let defaults = UserDefaults.standard
        guard let key = defaults.string(forKey: "naviaHoverRun"), let out = defaults.string(forKey: "naviaHoverOut") else { return }
        Task { @MainActor in
            try? await Task.sleep(for: .seconds(4))
            var phases: [String: [[Double]]] = [:]
            func phase(_ name: String, _ change: () -> Void) async {
                samples.removeAll()
                let start = CACurrentMediaTime()
                change()
                try? await Task.sleep(for: .milliseconds(700))
                phases[name] = samples.filter { $0.key == key }.map { [($0.time - start) * 1000, $0.scale] }
            }
            await phase("hover") { shared.hovered.insert(key) }
            await phase("press") { shared.pressed.insert(key) }
            await phase("release") { shared.pressed.remove(key) }
            await phase("leave") { shared.hovered.remove(key) }
            if let data = try? JSONSerialization.data(withJSONObject: phases, options: [.sortedKeys]) { try? data.write(to: URL(fileURLWithPath: out)) }
            NSApp.terminate(nil)
        }
    }
}

/// Sees every animated value of a grow's scale. SwiftUI sets `animatableData` once per frame.
struct GrowProbe: ViewModifier, Animatable {
    var scale: Double
    let key: String?
    nonisolated var animatableData: Double {
        get { scale }
        set {
            scale = newValue
            if let key { HoverLive.samples.append((CACurrentMediaTime(), key, newValue)) }
        }
    }
    func body(content: Content) -> some View { content }
}
#endif
