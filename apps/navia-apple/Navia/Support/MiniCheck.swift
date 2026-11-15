#if DEBUG
import AppKit

/// CoreCheck's mini player steps.
@MainActor enum MiniCheck {
    static func report(_ name: String, shell: ShellModel) -> [String: Any] {
        let mini = MiniPlayer.shared
        var out: [String: Any] = ["miniReport": name, "open": mini.isOpen, "size": NSStringFromSize(mini.size),
                                  "layout": MiniSize.stacked(mini.size) ? "stacked" : "compact",
                                  "mainVisible": mini.mainWindow?.isVisible ?? false, "viewsLive": shell.windowShown || shell.miniPlayerShown,
                                  "playing": shell.player.isPlaying, "position": shell.player.position,
                                  "song": shell.player.current?.title ?? "", "route": "\(shell.route)"]
        if let panel = mini.panel {
            out["panelVisible"] = panel.isVisible
            out["level"] = panel.level.rawValue
            out["floatingLevel"] = NSWindow.Level.floating.rawValue
            out["collectionBehavior"] = panel.collectionBehavior.rawValue
            out["canJoinAllSpaces"] = panel.collectionBehavior.contains(.canJoinAllSpaces)
            out["fullScreenAuxiliary"] = panel.collectionBehavior.contains(.fullScreenAuxiliary)
            out["nonactivating"] = panel.styleMask.contains(.nonactivatingPanel)
            out["hidesOnDeactivate"] = panel.hidesOnDeactivate
            out["frame"] = NSStringFromRect(panel.frame)
            out["contentMinSize"] = NSStringFromSize(panel.contentMinSize)
            out["contentMaxSize"] = NSStringFromSize(panel.contentMaxSize)
            out["resizable"] = panel.styleMask.contains(.resizable)
            out["visibleFrame"] = panel.screen.map { NSStringFromRect($0.visibleFrame) } ?? ""
            out["windowNumber"] = panel.windowNumber
        }
        return out
    }

    /// Every right-click menu in the app's windows, with its window, frame and item titles. The menus aren't shown.
    static func rightClickMenus() -> [String: Any] {
        func find(_ view: NSView) -> [RightClickMenuView] { ((view as? RightClickMenuView).map { [$0] } ?? []) + view.subviews.flatMap(find) }
        var out: [[String: Any]] = []
        for window in NSApp.windows where window.isVisible {
            guard let root = window.contentView?.superview ?? window.contentView else { continue }
            for view in find(root) {
                let titles = makeMenu(view.entries()).items.map { $0.isSeparatorItem ? "—" : $0.title }
                out.append(["window": window.windowNumber, "frame": NSStringFromRect(view.convert(view.bounds, to: nil)), "items": titles])
            }
        }
        return ["rightClickMenus": out]
    }

    /// `X:Y:DX:DY` sends a left drag into the panel from its top left, in steps, as the window server would while the
    /// panel moves under the pointer. Reports the view the press hit and the frame before and after.
    static func drag(_ value: String) -> [String: Any] {
        let v = value.split(separator: ":").compactMap { Double($0) }
        guard v.count == 4, let panel = MiniPlayer.shared.panel else { return ["miniDrag": value, "error": "no panel"] }
        let before = panel.frame
        let start = panel.convertPoint(toScreen: NSPoint(x: v[0], y: panel.frame.height - v[1]))
        let hit = panel.contentView?.hitTest(panel.convertPoint(fromScreen: start)).map { String(describing: type(of: $0)) } ?? "none"
        let steps = 12
        func send(_ type: NSEvent.EventType, _ screen: NSPoint) {
            guard let event = NSEvent.mouseEvent(with: type, location: panel.convertPoint(fromScreen: screen), modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                                                 windowNumber: panel.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: type == .leftMouseUp ? 0 : 1) else { return }
            panel.sendEvent(event)
        }
        send(.leftMouseDown, start)
        for i in 1...steps {
            send(.leftMouseDragged, NSPoint(x: start.x + v[2] * Double(i) / Double(steps), y: start.y - v[3] * Double(i) / Double(steps)))
        }
        send(.leftMouseUp, NSPoint(x: start.x + v[2], y: start.y - v[3]))
        return ["miniDrag": value, "hit": hit, "before": NSStringFromRect(before), "after": NSStringFromRect(panel.frame)]
    }

    /// `X:Y` sends a left click into the panel from its top left.
    static func click(_ value: String) -> Bool {
        let v = value.split(separator: ":").compactMap { Double($0) }
        guard v.count == 2, let panel = MiniPlayer.shared.panel else { return false }
        let point = NSPoint(x: v[0], y: panel.frame.height - v[1])
        for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
            guard let event = NSEvent.mouseEvent(with: type, location: point, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                                                 windowNumber: panel.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: type == .leftMouseDown ? 1 : 0)
            else { return false }
            panel.sendEvent(event)
        }
        return true
    }

    /// Every main-menu item on Command-M, to check the shortcut against Minimize All.
    static func menuItems() -> [String] {
        var out: [String] = []
        func walk(_ menu: NSMenu, _ path: String) {
            for item in menu.items {
                if item.keyEquivalent.lowercased() == "m", item.keyEquivalentModifierMask.contains(.command) {
                    out.append("\(path)/\(item.title) mods=\(item.keyEquivalentModifierMask.rawValue) alt=\(item.isAlternate) hidden=\(item.isHidden)")
                }
                if let sub = item.submenu { walk(sub, path + "/" + item.title) }
            }
        }
        if let main = NSApp.mainMenu { walk(main, "") }
        return out
    }
}
#endif
