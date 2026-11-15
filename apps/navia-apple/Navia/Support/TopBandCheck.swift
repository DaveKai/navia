#if DEBUG
import AppKit

/// CoreCheck steps for the top band, where content scrolls under the top bar and the title area. The window server
/// checks real clicks there against AppKit's drag region, which `window.sendEvent` never sees, so these read
/// AppKit's own answers.
@MainActor enum TopBandCheck {
    private typealias RegionFn = @convention(c) (AnyObject, Selector, CGRect) -> OpaquePointer?
    private typealias EventBoolFn = @convention(c) (AnyObject, Selector, NSEvent) -> Bool
    private typealias RectFn = @convention(c) (AnyObject, Selector) -> CGRect
    private typealias EnumFn = @convention(c) (OpaquePointer) -> OpaquePointer?
    private typealias NextFn = @convention(c) (OpaquePointer) -> UnsafePointer<CGRect>?
    private typealias ReleaseFn = @convention(c) (OpaquePointer) -> Void

    private static func symbol<T>(_ name: String, _ type: T.Type) -> T? {
        guard let handle = dlopen(nil, RTLD_NOW), let sym = dlsym(handle, name) else { return nil }
        return unsafeBitCast(sym, to: type)
    }

    /// The rects of opaque views that block window dragging, as AppKit computes them for the window server.
    static func blockingRects(_ window: NSWindow) -> [[Int]]? {
        let sel = NSSelectorFromString("_regionForOpaqueViewsBlockingDraggableFrame:")
        guard window.responds(to: sel), let enumerate = symbol("CGSRegionEnumerator", EnumFn.self),
              let next = symbol("CGSNextRect", NextFn.self) else { return nil }
        let fn = unsafeBitCast(window.method(for: sel), to: RegionFn.self)
        let frame = window.contentView?.superview?.bounds ?? .zero
        guard let region = fn(window, sel, frame), let it = enumerate(region) else { return [] }
        var rects: [[Int]] = []
        while let r = next(it), rects.count < 400 { rects.append([Int(r.pointee.minX), Int(r.pointee.minY), Int(r.pointee.width), Int(r.pointee.height)]) }
        symbol("CGSReleaseRegionEnumerator", ReleaseFn.self)?(it)
        symbol("CGSReleaseRegion", ReleaseFn.self)?(region)
        return rects
    }

    /// `dragProbe=X:Y0:Y1:STEP`: down the window at X, the view chain a click reaches, each view's
    /// `mouseDownCanMoveWindow`, and whether AppKit would start a window drag there.
    static func dragProbe(_ value: String, window: NSWindow) -> [String: Any] {
        let v = value.split(separator: ":").compactMap { Double($0) }
        guard v.count == 4, let content = window.contentView, let root = content.superview else { return ["dragProbe": "bad"] }
        let height = content.bounds.height
        let dragSel = NSSelectorFromString("_shouldStartWindowDragForEvent:")
        let shouldDrag = window.responds(to: dragSel) ? unsafeBitCast(window.method(for: dragSel), to: EventBoolFn.self) : nil
        let opaqueSel = NSSelectorFromString("_opaqueRectForWindowMoveWhenInTitlebar")
        var samples: [[String: Any]] = []
        var y = v[1]
        while y <= v[2] {
            let p = NSPoint(x: v[0], y: height - y)
            let hit = root.hitTest(root.convert(p, from: nil))
            var chain: [String] = []
            var h = hit
            while let c = h, chain.count < 6 {
                var entry = String(describing: type(of: c)).prefix(48) + (c.mouseDownCanMoveWindow ? "+m" : "-m")
                if c.responds(to: opaqueSel) {
                    let r = unsafeBitCast(c.method(for: opaqueSel), to: RectFn.self)(c, opaqueSel)
                    if !r.isEmpty { entry += "[\(Int(r.width))x\(Int(r.height))]" }
                }
                chain.append(String(entry)); h = c.superview
            }
            var drag: Any = "?"
            if let shouldDrag, let e = NSEvent.mouseEvent(with: .leftMouseDown, location: p, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                                                         windowNumber: window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1) {
                drag = shouldDrag(window, dragSel, e)
            }
            // Tracking areas are clipped to the hit view's visible rect, so hover needs the point inside it.
            let visible = hit.map { $0.visibleRect.contains($0.convert(p, from: nil)) } ?? false
            samples.append(["y": y, "drag": drag, "hit": chain.joined(separator: "<"), "vis": visible])
            y += v[3]
        }
        let titlebar = root.subviews.first { String(describing: type(of: $0)).contains("Titlebar") }
        return ["dragProbe": value, "titlebarH": titlebar.map { Double($0.frame.height) } ?? -1,
                "layoutTop": Double(height - window.contentLayoutRect.maxY), "contentH": Double(height),
                "blocking": blockingRects(window) ?? "unavailable", "samples": samples]
    }

    /// `layerDump=X:Y`: every layer whose frame holds the window point, outermost first, with its class, owner,
    /// opacity, filters and mask, and for backdrop layers what they sample and who renders them.
    static func layerDump(_ value: String, window: NSWindow) -> [String: Any] {
        if value.hasPrefix("band") { return bandAudit(value, window: window) }
        if value.hasPrefix("hide:") || value == "unhide" { return hideLayers(value, window: window) }
        // `layerDump=material:N` sets the behind-window blur's material, and `layerDump=wsblur:R` the window server's blur
        // radius, for comparing how blurry the desktop looks.
        if value == "backing:on" || value == "backing:off" { return backing(value == "backing:on", main: window) }
        if value.hasPrefix("material:") || value.hasPrefix("wsblur:") {
            let n = Int(value.split(separator: ":").last ?? "") ?? 0
            func find(_ v: NSView) -> WindowBlurView? { (v as? WindowBlurView) ?? v.subviews.lazy.compactMap(find).first }
            guard let root = window.contentView?.superview, let blur = find(root) else { return ["blur": "none"] }
            if value.hasPrefix("material:") { blur.material = NSVisualEffectView.Material(rawValue: n) ?? blur.material }
            else { WindowServerBlur.set(window, radius: n) }
            return ["blur": value, "material": blur.material.rawValue]
        }
        let v = value.split(separator: ":").compactMap { Double($0) }
        guard v.count == 2, let root = window.contentView?.superview?.layer else { return ["layerDump": "bad"] }
        let height = window.contentView?.superview?.bounds.height ?? 0
        var found: [String] = []
        func walk(_ layer: CALayer, depth: Int) {
            let r = layer.convert(layer.bounds, to: root)
            // Whether the root layer is flipped depends on the view, so test both readings.
            let p1 = CGPoint(x: v[0], y: height - v[1]), p2 = CGPoint(x: v[0], y: v[1])
            let holds = r.contains(p1) || r.contains(p2)
            if holds {
                let name = String(describing: type(of: layer))
                let owner = layer.delegate.map { String(describing: type(of: $0)).prefix(50) } ?? ""
                var s = "\(String(repeating: " ", count: min(depth, 40)))\(name) \(owner) \(Int(r.minX)),\(Int(r.minY)) \(Int(r.width))x\(Int(r.height)) o=\(layer.opacity)\(layer.isHidden ? " HIDDEN" : "")"
                if let f = layer.filters, !f.isEmpty { s += " filters=\(f.map { "\(($0 as AnyObject).value(forKey: "name") ?? $0)" })" }
                if let f = layer.backgroundFilters, !f.isEmpty { s += " bgFilters=\(f.map { "\(($0 as AnyObject).value(forKey: "name") ?? $0)" })" }
                if let c = layer.compositingFilter { s += " comp=\(c)" }
                if layer.mask != nil { s += " mask" }
                if layer.masksToBounds { s += " clips" }
                if name.contains("Backdrop") {
                    for key in ["groupName", "enabled", "windowServerAware", "captureOnly", "scale", "marginWidth", "ignoresOffscreenGroups", "allowsInPlaceFiltering", "zoom", "reducesCaptureBitDepth"] {
                        if layer.responds(to: NSSelectorFromString(key)) { s += " \(key)=\(layer.value(forKey: key) ?? "nil")" }
                    }
                }
                found.append(s)
            }
            for sub in layer.sublayers ?? [] { walk(sub, depth: depth + 1) }
        }
        walk(root, depth: 0)
        return ["layerDump": value, "layers": found]
    }

    /// `layerDump=band[:H]`: every backdrop layer in the app's visible windows that reaches into the top H points
    /// (default 140), with who renders it and whether it draws at all, plus the views that make materials and edges.
    static func bandAudit(_ value: String, window main: NSWindow) -> [String: Any] {
        let band = value.split(separator: ":").dropFirst().first.flatMap { Double($0) } ?? 140
        var backdrops: [String] = [], views: [String] = []
        for window in NSApp.windows where window.isVisible {
            guard let rootView = window.contentView?.superview, let root = rootView.layer else { continue }
            let height = rootView.bounds.height
            let tag: String = window === main ? "main" : "\(String(describing: type(of: window)).prefix(30))#\(window.windowNumber)"
            func top(_ r: CGRect, flipped: Bool) -> (CGFloat, CGFloat) { flipped ? (r.minY, r.maxY) : (height - r.maxY, height - r.minY) }
            func walk(_ layer: CALayer, drawn: Bool, owners: [String]) {
                let shows = drawn && !layer.isHidden && layer.opacity > 0
                var chain = owners
                if let d = layer.delegate { chain.append(String(String(describing: type(of: d)).prefix(40))) }
                let name = String(describing: type(of: layer))
                if name.contains("Backdrop") {
                    let r = layer.convert(layer.bounds, to: root)
                    let (y0, y1) = top(r, flipped: root.isGeometryFlipped)
                    if y0 < band, r.width > 0, r.height > 0 {
                        var s = "\(tag) \(name) y=\(Int(y0))..\(Int(y1)) x=\(Int(r.minX)) w=\(Int(r.width)) \(shows ? "DRAWN" : "off")"
                        for key in ["windowServerAware", "enabled", "groupName"] where layer.responds(to: NSSelectorFromString(key)) {
                            s += " \(key)=\(layer.value(forKey: key) ?? "nil")"
                        }
                        s += " in=" + chain.suffix(4).joined(separator: "<")
                        backdrops.append(s)
                    }
                }
                for sub in layer.sublayers ?? [] { walk(sub, drawn: shows, owners: chain) }
            }
            walk(root, drawn: true, owners: [])
            func walkViews(_ view: NSView) {
                let name = String(describing: type(of: view))
                if view is NSVisualEffectView || ["Glass", "Pocket", "Titlebar", "Toolbar", "Edge", "Material", "Blur"].contains(where: name.contains) {
                    let r = view.convert(view.bounds, to: nil)
                    let y0 = height - r.maxY
                    if y0 < band {
                        var s = "\(tag) \(name.prefix(60)) y=\(Int(y0))..\(Int(height - r.minY)) x=\(Int(r.minX)) w=\(Int(r.width))\(view.isHiddenOrHasHiddenAncestor ? " hidden" : "")"
                        if let e = view as? NSVisualEffectView { s += " material=\(e.material.rawValue) blending=\(e.blendingMode.rawValue) state=\(e.state.rawValue)" }
                        views.append(s)
                    }
                }
                view.subviews.forEach(walkViews)
            }
            walkViews(rootView)
        }
        let st = InAppBackdrops.stats
        return ["layerDump": value, "backdrops": backdrops, "views": views,
                "sweeps": ["count": st.sweeps, "layers": st.layers, "flipped": st.flipped, "lastMicros": Int(st.lastMicros), "maxMicros": Int(st.maxMicros)]]
    }

    private static var backingWindow: NSWindow?
    /// `layerDump=backing:on|off`: a window of busy stripes and color blocks right behind the main one, so the
    /// behind-window blur has something to blur whatever the desktop shows.
    static func backing(_ on: Bool, main: NSWindow) -> [String: Any] {
        backingWindow?.orderOut(nil)
        backingWindow = nil
        guard on else { return ["backing": "off"] }
        let w = NSWindow(contentRect: main.frame, styleMask: [.borderless], backing: .buffered, defer: false)
        w.isReleasedWhenClosed = false
        let image = NSImage(size: main.frame.size, flipped: false) { rect in
            let colors: [NSColor] = [.systemRed, .systemYellow, .systemGreen, .systemBlue, .white, .black]
            var x: CGFloat = 0, i = 0
            while x < rect.width {
                colors[i % colors.count].setFill()
                NSRect(x: x, y: 0, width: i % 3 == 0 ? 6 : 40, height: rect.height).fill()
                x += i % 3 == 0 ? 6 : 40; i += 1
            }
            NSColor.white.setFill()
            var y: CGFloat = 0
            while y < rect.height { NSRect(x: 0, y: y, width: rect.width, height: 2).fill(); y += 24 }
            return true
        }
        let view = NSImageView(image: image)
        view.imageScaling = .scaleNone
        w.contentView = view
        w.order(.below, relativeTo: main.windowNumber)
        backingWindow = w
        return ["backing": "on", "window": w.windowNumber]
    }

    private static var hidden: [CALayer] = []
    /// `layerDump=hide:PATTERN:X0:X1`: hides every layer whose class or owner's class contains PATTERN and whose frame
    /// starts within X0...X1, to find what draws a mark. `layerDump=unhide` shows them again.
    static func hideLayers(_ value: String, window: NSWindow) -> [String: Any] {
        if value == "unhide" {
            hidden.forEach { $0.isHidden = false }
            defer { hidden = [] }
            return ["unhidden": hidden.count]
        }
        let parts = value.split(separator: ":").map(String.init)
        guard parts.count == 4, let x0 = Double(parts[2]), let x1 = Double(parts[3]),
              let root = window.contentView?.superview?.layer else { return ["hide": "bad"] }
        var names: [String] = []
        func walk(_ layer: CALayer) {
            let name = String(describing: type(of: layer))
            let owner = layer.delegate.map { String(describing: type(of: $0)) } ?? ""
            let r = layer.convert(layer.bounds, to: root)
            if (name.contains(parts[1]) || owner.contains(parts[1])), r.minX >= x0, r.minX <= x1, !layer.isHidden {
                layer.isHidden = true
                hidden.append(layer)
                names.append("\(name) \(owner.prefix(40)) \(Int(r.minX)),\(Int(r.minY)) \(Int(r.width))x\(Int(r.height))")
                return
            }
            layer.sublayers?.forEach(walk)
        }
        walk(root)
        return ["hide": value, "layers": names]
    }

    private static var watcher: Any?
    private static var watched: [String] = []
    /// `eventWatch=on|off`: records every mouse down, up and move the app receives, with its window point and the view
    /// chain it hits, and reports them on `off`.
    static func eventWatch(_ value: String, window: NSWindow) -> [String: Any]? {
        if value == "on" {
            watched = []
            watcher = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .leftMouseUp, .mouseMoved, .leftMouseDragged]) { e in
                MainActor.assumeIsolated {
                    guard watched.count < 400, let content = window.contentView, let root = content.superview else { return }
                    let p = e.locationInWindow
                    var chain: [String] = []
                    var h = e.window === window ? root.hitTest(root.convert(p, from: nil)) : nil
                    while let c = h, chain.count < 3 { chain.append(String(String(describing: type(of: c)).prefix(40))); h = c.superview }
                    watched.append("\(e.type.rawValue) \(Int(p.x)),\(Int(content.bounds.height - p.y)) key=\(window.isKeyWindow ? 1 : 0) \(chain.joined(separator: "<"))")
                }
                return e
            }
            return nil
        }
        if let watcher { NSEvent.removeMonitor(watcher) }
        watcher = nil
        return ["eventWatch": watched]
    }

    /// `postClick=X:Y`: a mouse down and up posted to this process with `CGEventPostToPid`, so AppKit routes it like a
    /// real click. The pointer doesn't move. Reports the route before and after.
    static func postClick(_ value: String, window: NSWindow, route: () -> String) async -> [String: Any] {
        let v = value.split(separator: ":").compactMap { Double($0) }
        guard v.count == 2, let screen = NSScreen.screens.first else { return ["postClick": "bad"] }
        let content = window.contentRect(forFrameRect: window.frame)
        let global = CGPoint(x: content.minX + v[0], y: screen.frame.height - content.maxY + v[1])
        let before = route()
        var seen: [String] = []
        let monitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown, .leftMouseUp]) { e in
            seen.append("\(e.type == .leftMouseDown ? "down" : "up") w=\(e.window === window ? 1 : 0)")
            return e
        }
        for type in [CGEventType.leftMouseDown, .leftMouseUp] {
            guard let e = CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: global, mouseButton: .left) else { continue }
            e.setIntegerValueField(.mouseEventClickState, value: 1)
            e.setIntegerValueField(.mouseEventWindowUnderMousePointer, value: Int64(window.windowNumber))
            e.setIntegerValueField(.mouseEventWindowUnderMousePointerThatCanHandleThisEvent, value: Int64(window.windowNumber))
            e.postToPid(getpid())
            try? await Task.sleep(for: .milliseconds(60))
        }
        try? await Task.sleep(for: .milliseconds(400))
        if let monitor { NSEvent.removeMonitor(monitor) }
        return ["postClick": value, "routeBefore": before, "routeAfter": route(), "seen": seen]
    }
}
#endif
