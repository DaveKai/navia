#if DEBUG
import AppKit
import SwiftUI

/// Reduce Motion checks without a screen capture. `motionSample=NAME:X:Y:MS` samples what moves every frame for MS
/// milliseconds: the scroll view under X:Y, the table's drop slot and rows, and the expanded player's clip.
/// `motionReport=NAME` reports how many distinct values each passed through. A glide passes through many, and
/// under reduced motion each jumps in one step.
@MainActor enum MotionCheck {
    /// Set by the expanded player's clip as SwiftUI interpolates it.
    static var clipProgress: Double?
    private static var samples: [String: [String: [(Double, String)]]] = [:]
    private static var timer: Timer?

    static func start(_ value: String, window: NSWindow) {
        let parts = value.split(separator: ":").map(String.init)
        guard parts.count == 4, let x = Double(parts[1]), let y = Double(parts[2]), let ms = Double(parts[3]) else { return }
        let name = parts[0]
        let point = NSPoint(x: x, y: (window.contentView?.bounds.height ?? 0) - y)
        let root = window.contentView?.superview
        var hit = root.flatMap { $0.hitTest($0.convert(point, from: nil)) }
        while let view = hit, !(view is NSScrollView) { hit = view.superview }
        let scroll = hit as? NSScrollView
        samples[name] = [:]
        if let scroll { record(name, "scrollView", 0, "\(scroll.convert(scroll.bounds, to: nil)) doc \(scroll.documentView?.frame.height ?? 0)") }
        let start = CACurrentMediaTime()
        clipProgress = nil
        timer?.invalidate()
        let ticker = Timer(timeInterval: 1.0 / 120, repeats: true) { _ in
            MainActor.assumeIsolated {
                let t = CACurrentMediaTime() - start
                if t * 1000 > ms { MotionCheck.timer?.invalidate(); MotionCheck.timer = nil; return }
                if let scroll { record(name, "scroll", t, String(format: "%.1f", scroll.contentView.bounds.minY)) }
                if let root, let slot = find(DropSlotView.self, in: root), !slot.isHidden {
                    let layer = slot.layer?.presentation() ?? slot.layer
                    if let layer { record(name, "slot", t, String(format: "%.1f %.1f %.2f", layer.frame.minY, layer.frame.height, layer.opacity)) }
                }
                if let root, let table = find(NSTableView.self, in: root) {
                    let range = table.rows(in: table.visibleRect)
                    let ys = (range.location..<min(range.location + 8, range.location + range.length)).compactMap { row in
                        table.rowView(atRow: row, makeIfNecessary: false).flatMap { ($0.layer?.presentation() ?? $0.layer).map { String(format: "%.0f", $0.frame.minY) } }
                    }
                    record(name, "tableRows", t, ys.joined(separator: " "))
                }
                if let clipProgress { record(name, "expandClip", t, String(format: "%.3f", clipProgress)) }
            }
        }
        timer = ticker
        RunLoop.main.add(ticker, forMode: .common)
    }

    /// `expandAnimated=1|0` opens or closes the expanded player the way its buttons do.
    static func expand(_ shell: ShellModel, _ on: Bool) {
        withAnimation(ExpandClip.animation) { shell.expanded = on }
    }

    static func report(_ name: String) -> [String: Any] {
        var out: [String: Any] = ["motionReport": name, "reduced": Motion.isReduced]
        for (probe, values) in samples[name] ?? [:] {
            var distinct: [String] = []
            for (_, v) in values where distinct.last != v { distinct.append(v) }
            let changes = zip(values, values.dropFirst()).filter { $0.1 != $1.1 }.map { $1.0 }
            out[probe] = ["steps": distinct.count, "first": distinct.first ?? "", "last": distinct.last ?? "",
                          "changingMs": Int(((changes.last ?? 0) - (changes.first ?? 0)) * 1000), "samples": values.count]
        }
        return out
    }

    private static func record(_ name: String, _ probe: String, _ t: Double, _ value: String) {
        samples[name, default: [:]][probe, default: []].append((t, value))
    }

    private static func find<V: NSView>(_ type: V.Type, in view: NSView) -> V? {
        if let match = view as? V { return match }
        for sub in view.subviews { if let match = find(type, in: sub) { return match } }
        return nil
    }
}
#endif
