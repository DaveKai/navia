#if DEBUG
import AppKit
import SwiftUI

/// CoreCheck steps for the account button and menu icons (`account=<what>`), run inside the app's own window with
/// no real input:
/// - `buttons`: each account button's frame and what a click at its center reaches.
/// - `menu`: the account menu's rows, then flips Private Mode through its switch row and back.
/// - `open:SECONDS`: pops the account menu up and closes it after SECONDS, so an outside capture can see it.
/// - `songMenu`: the first library song's menu rows and whether each has an icon.
/// - `settingsKey`: sends ⌘, to the app menu and reports the route before and after.
@MainActor enum AccountCheck {
    static func run(_ value: String, shell: ShellModel, window: NSWindow) async -> [String: Any] {
        let parts = value.split(separator: ":").map(String.init)
        switch parts.first ?? "" {
        case "buttons": return buttons(window, shell: shell)
        case "menu": return menuReport(shell)
        case "open": return await open(window, seconds: Double(parts.dropFirst().first ?? "") ?? 5, which: parts.count > 2 ? parts[2] : "account", shell: shell)
        case "songMenu": return songMenu(shell)
        case "iconsPng": return iconsPng(parts.dropFirst().joined(separator: ":"), shell: shell)
        case "settingsKey": return await settingsKey(shell)
        default: return ["account": "unknown \(value)"]
        }
    }

    private static func all(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(all) }

    private static func topLeft(_ view: NSView, _ window: NSWindow) -> [Int] {
        let r = view.convert(view.bounds, to: nil)
        let h = window.contentView?.bounds.height ?? 0
        return [Int(r.minX), Int(h - r.maxY), Int(r.width), Int(r.height)]
    }

    private static func accountViews(_ window: NSWindow) -> [MenuOpeningView] {
        guard let root = window.contentView else { return [] }
        return all(root).compactMap { $0 as? MenuOpeningView }.filter { $0.trailing && $0.window != nil && !$0.isHiddenOrHasHiddenAncestor }
    }

    private static func buttons(_ window: NSWindow, shell: ShellModel) -> [String: Any] {
        // A real click starts at the theme frame, the content view's superview, above the toolbar's band.
        let root = window.contentView?.superview ?? window.contentView!
        let list: [[String: Any]] = accountViews(window).map { view in
            let r = view.convert(view.bounds, to: nil)
            let hit = root.hitTest(root.convert(NSPoint(x: r.midX, y: r.midY), from: nil))
            return ["frame": topLeft(view, window), "hitIsButton": hit === view, "hitView": hit.map { String(describing: type(of: $0)) } ?? "none"]
        }
        return ["accountButtons": list, "expanded": shell.expanded, "route": "\(shell.route)", "windowWidth": Int(window.frame.width)]
    }

    private static func describe(_ menu: NSMenu) -> [[String: Any]] {
        menu.items.map { item in
            if item.isSeparatorItem { return ["divider": true] }
            if let row = item.view as? MenuSwitchRow { return ["switch": true, "on": row.debugOn] }
            if item.view != nil { return ["view": String(describing: type(of: item.view!))] }
            var hasIcon = false
            item.attributedTitle?.enumerateAttribute(.attachment, in: NSRange(location: 0, length: item.attributedTitle?.length ?? 0)) { v, _, _ in if v != nil { hasIcon = true } }
            var row: [String: Any] = ["title": item.title, "icon": hasIcon, "enabled": item.isEnabled]
            if !item.keyEquivalent.isEmpty { row["key"] = item.keyEquivalent; row["command"] = item.keyEquivalentModifierMask.contains(.command) }
            if let tip = item.toolTip { row["reason"] = tip }
            return row
        }
    }

    private static func menuReport(_ shell: ShellModel) -> [String: Any] {
        let before = shell.player.privateMode
        let menu = makeMenu(AccountMenu.entries(shell: shell))
        let rows = describe(menu)
        let row = menu.items.compactMap { $0.view as? MenuSwitchRow }.first
        row?.debugFlip()
        let flipped = shell.player.privateMode
        let switchShows = row?.debugOn
        row?.debugFlip()
        return ["accountMenu": rows, "privateBefore": before, "privateAfterFlip": flipped, "switchShowsAfterFlip": switchShows as Any,
                "privateAfterSecondFlip": shell.player.privateMode, "menuStillHasItems": menu.items.count]
    }

    /// `which`: `account` (the account button), `plain` (its rows without the custom views), or `more` (the first other
    /// menu button).
    private static func open(_ window: NSWindow, seconds: Double, which: String, shell: ShellModel) async -> [String: Any] {
        let others = (window.contentView.map(all) ?? []).compactMap { $0 as? MenuOpeningView }.filter { !$0.trailing && !$0.isHiddenOrHasHiddenAncestor }
        guard let view = which == "more" ? others.last : accountViews(window).first else { return ["accountOpen": "no button"] }
        if which == "plain" { let keep = view.entries; view.entries = { AccountMenu.entries(shell: shell).filter { $0.view == nil && !$0.isSwitch } }; defer { view.entries = keep } ; return await openNow(view, window, seconds) }
        return await openNow(view, window, seconds)
    }

    private static func openNow(_ view: MenuOpeningView, _ window: NSWindow, _ seconds: Double) async -> [String: Any] {
        final class Seen: @unchecked Sendable { var frame: [Int] = []; var windowNumber = 0 }
        let seen = Seen()
        let h = window.screen?.frame.maxY ?? 0
        // The menu tracks in its own run loop mode, and this timer runs in common modes, so it fires while it's open.
        let timer = Timer(timeInterval: seconds, repeats: false) { _ in
            MainActor.assumeIsolated {
                if let menuWindow = NSApp.windows.first(where: { $0.isVisible && String(describing: type(of: $0)).contains("Menu") }) {
                    let f = menuWindow.frame
                    seen.frame = [Int(f.minX), Int(h - f.maxY), Int(f.width), Int(f.height)]
                    seen.windowNumber = menuWindow.windowNumber
                }
                MenuOpeningView.lastOpened?.cancelTracking()
            }
        }
        RunLoop.main.add(timer, forMode: .common)
        let b = view.convert(view.bounds, to: nil)
        let screenButton = window.convertToScreen(b)
        view.open()
        return ["accountOpenMenuFrame": seen.frame, "menuWindowNumber": seen.windowNumber,
                "buttonScreenFrame": [Int(screenButton.minX), Int(h - screenButton.maxY), Int(screenButton.width), Int(screenButton.height)]]
    }

    /// Draws the account and song menus' row titles into an off-screen PNG in dark appearance, to check the icons take
    /// the text color.
    private static func iconsPng(_ path: String, shell: ShellModel) -> [String: Any] {
        var titles = makeMenu(AccountMenu.entries(shell: shell)).items.compactMap(\.attributedTitle)
        if let song = shell.songs.song(at: 0) { titles += makeMenu(SongMenu.single(song, shell: shell)).items.compactMap(\.attributedTitle) }
        let size = NSSize(width: 300, height: CGFloat(titles.count) * 26 + 12)
        let image = NSImage(size: size)
        NSAppearance(named: .darkAqua)!.performAsCurrentDrawingAppearance {
            image.lockFocus()
            NSColor(white: 0.16, alpha: 1).setFill(); NSRect(origin: .zero, size: size).fill()
            for (i, t) in titles.enumerated() { t.draw(at: NSPoint(x: 12, y: size.height - CGFloat(i + 1) * 26)) }
            image.unlockFocus()
        }
        if let tiff = image.tiffRepresentation, let rep = NSBitmapImageRep(data: tiff), let png = rep.representation(using: .png, properties: [:]) {
            try? png.write(to: URL(fileURLWithPath: path))
        }
        return ["iconsPng": path, "rows": titles.count]
    }

    private static func songMenu(_ shell: ShellModel) -> [String: Any] {
        guard let song = shell.songs.song(at: 0) else { return ["songMenu": "no song"] }
        return ["songMenu": describe(makeMenu(SongMenu.single(song, shell: shell))),
                "severalMenu": describe(makeMenu(SongMenu.several([song], shell: shell)))]
    }

    private static func settingsKey(_ shell: ShellModel) async -> [String: Any] {
        let before = ["route": "\(shell.route)", "expanded": shell.expanded] as [String: Any]
        let item = NSApp.mainMenu?.items.first?.submenu?.items.first { $0.keyEquivalent == "," }
        let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: .command, timestamp: ProcessInfo.processInfo.systemUptime,
                                     windowNumber: NSApp.mainWindow?.windowNumber ?? 0, context: nil, characters: ",", charactersIgnoringModifiers: ",",
                                     isARepeat: false, keyCode: 43)
        let handled = event.map { NSApp.mainMenu?.performKeyEquivalent(with: $0) ?? false } ?? false
        try? await Task.sleep(for: .milliseconds(400))
        return ["settingsKeyBefore": before, "appMenuItem": item?.title ?? "missing", "handled": handled,
                "settingsKeyAfter": ["route": "\(shell.route)", "expanded": shell.expanded]]
    }
}
#endif
