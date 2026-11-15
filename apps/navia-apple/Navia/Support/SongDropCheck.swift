#if DEBUG
import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// A stand-in dragging session carrying songs, for `songDrop`. It goes through the window's own
/// NSDraggingDestination methods, so SwiftUI's drop handling runs as for a real drag.
@MainActor final class SongDropInfo: NSObject, NSDraggingInfo {
    private let window: NSWindow
    var location: NSPoint
    private let source: Any?
    private let pasteboard = NSPasteboard(name: NSPasteboard.Name("dev.navia.songdropcheck"))

    init(window: NSWindow, location: NSPoint, source: Any?, songIDs: [String]) {
        self.window = window
        self.location = location
        self.source = source
        super.init()
        pasteboard.clearContents()
        pasteboard.writeObjects(songIDs.map { id in let item = NSPasteboardItem(); item.setString(id, forType: .naviaSongs); return item })
    }

    var draggingDestinationWindow: NSWindow? { window }
    var draggingSourceOperationMask: NSDragOperation { [.copy, .move] }
    var draggingLocation: NSPoint { location }
    var draggedImageLocation: NSPoint { location }
    var draggedImage: NSImage? { nil }
    var draggingPasteboard: NSPasteboard { pasteboard }
    var draggingSource: Any? { source }
    var draggingSequenceNumber: Int { 4343 }
    func slideDraggedImage(to screenPoint: NSPoint) {}
    var draggingFormation: NSDraggingFormation = .none
    var animatesToDestination = false
    var numberOfValidItemsForDrop: Int = 1
    func enumerateDraggingItems(options enumOpts: NSDraggingItemEnumerationOptions = [], for view: NSView?, classes classArray: [AnyClass],
                                searchOptions: [NSPasteboard.ReadingOptionKey: Any] = [:], using block: (NSDraggingItem, Int, UnsafeMutablePointer<ObjCBool>) -> Void) {}
    var springLoadingHighlight: NSSpringLoadingHighlight { .none }
    func resetSpringLoading() {}
}

/// CoreCheck steps for songs dragged onto playlists and the queue.
@MainActor enum SongDropCheck {
    private static var open: (info: SongDropInfo, dest: NSView)?

    /// `songDrop=SOURCE|TARGET[|DY][|hold]`. SOURCE is `table:I+J` or `ids:ID+ID`. TARGET is a drop target's key
    /// (`playlist:ID`, `queue-end:tab`, `queue-end:bar`, `queue`), held DY points below its top or at its middle.
    /// `hold` leaves the drag open until `songDrop=end` or `songDrop=exit`. Reports what the target shows and what the
    /// drop did. With TARGET `sidebar-top` or `sidebar-bottom`, holds the songs at the sidebar's edge for a second and
    /// reports how far it scrolled.
    static func run(_ value: String, shell: ShellModel, window: NSWindow) async -> [String: Any] {
        let parts = value.split(separator: "|").map(String.init)
        if parts.first == "end" || parts.first == "exit" {
            guard let open else { return ["songDrop": "none open"] }
            if parts.first == "end" { await release(shell) }
            finish(drop: parts.first == "end", shell: shell, window: window, open: open)
            Self.open = nil
            try? await Task.sleep(for: .milliseconds(600))
            return report(shell, value: value, dest: open.dest)
        }
        guard parts.count >= 2 else { return ["songDrop": "bad"] }
        let songs = sourceSongs(parts[0], shell: shell, window: window)
        guard !songs.isEmpty else { return ["songDrop": "no songs"] }
        let table = parts[0].hasPrefix("table:") ? TableCheck.table(window) : nil
        if let table, let c = table.controller {
            c.debugBeginDrag(songs: parts[0].dropFirst(6).split(separator: "+").compactMap { Int($0) })
        }
        shell.beginSongDrag(songs)
        let height = window.contentView?.bounds.height ?? 0

        if parts[1] == "sidebar-top" || parts[1] == "sidebar-bottom" {
            guard let scroll = shell.songDrag.sidebarScroll else { return ["songDrop": "no sidebar"] }
            let box = scroll.convert(scroll.bounds, to: nil)
            let before = Double(scroll.contentView.bounds.origin.y)
            shell.songDrag.debugPointer = NSPoint(x: box.midX, y: parts[1] == "sidebar-top" ? box.maxY - 10 : box.minY + 10)
            try? await Task.sleep(for: .milliseconds(1000))
            let after = Double(scroll.contentView.bounds.origin.y)
            shell.songDrag.debugPointer = nil
            shell.songDrag.end()
            table?.controller?.debugEndDrag()
            return ["songDrop": value, "scrollBefore": before, "scrollAfter": after, "contentHeight": Double(scroll.documentView?.frame.height ?? 0)]
        }

        guard let frame = shell.songDrag.debugFrames[parts[1]] else {
            shell.songDrag.end()
            table?.controller?.debugEndDrag()
            return ["songDrop": "no target \(parts[1])", "targets": Array(shell.songDrag.debugFrames.keys).sorted()]
        }
        let dy = parts.count > 2 ? Double(parts[2]) : nil
        let topLeft = CGPoint(x: frame.midX, y: frame.minY + (dy.map { CGFloat($0) } ?? frame.height / 2))
        let point = NSPoint(x: topLeft.x, y: height - topLeft.y)
        shell.songDrag.debugPointer = point
        guard let dest = destination(at: point, in: window) else {
            shell.songDrag.end()
            table?.controller?.debugEndDrag()
            func registered(_ v: NSView) -> [String] {
                (v.registeredDraggedTypes.isEmpty || v is NSImageView ? [] : ["\(type(of: v)) \(v.convert(v.bounds, to: nil)) hidden=\(v.isHiddenOrHasHiddenAncestor)"]) + v.subviews.flatMap(registered)
            }
            return ["songDrop": "no destination", "point": [Double(topLeft.x), Double(topLeft.y)],
                    "registered": window.contentView?.superview.map(registered) ?? []]
        }
        let info = SongDropInfo(window: window, location: point, source: table, songIDs: songs.map(\.id))
        outsideTask { _ = (dest as any NSDraggingDestination).draggingEntered?(info) }
        for _ in 0..<5 {
            try? await Task.sleep(for: .milliseconds(60))
            outsideTask { _ = (dest as any NSDraggingDestination).draggingUpdated?(info) }
        }
        try? await Task.sleep(for: .milliseconds(450))
        var out = report(shell, value: value, dest: dest)
        out["point"] = [Double(topLeft.x), Double(topLeft.y)]
        out["frame"] = [Double(frame.minX), Double(frame.minY), Double(frame.width), Double(frame.height)]
        if parts.count > 3 && parts[3] == "hold" || parts.count > 2 && parts[2] == "hold" {
            open = (info, dest)
            return out
        }
        await release(shell)
        finish(drop: true, shell: shell, window: window, open: (info, dest))
        try? await Task.sleep(for: .milliseconds(800))
        out["after"] = report(shell, value: value, dest: dest)
        return out
    }

    /// The button goes up a few ticks before the drop arrives, as with a real drag, and the drag must still take it.
    private static func release(_ shell: ShellModel) async {
        shell.songDrag.debugPointer = nil
        try? await Task.sleep(for: .milliseconds(120))
    }

    private static func finish(drop: Bool, shell: ShellModel, window: NSWindow, open: (info: SongDropInfo, dest: NSView)) {
        let (info, dest) = open
        outsideTask {
            let d = dest as any NSDraggingDestination
            if drop, d.prepareForDragOperation?(info) ?? true { _ = d.performDragOperation?(info); d.concludeDragOperation?(info) } else { d.draggingExited?(info) }
            shell.songDrag.debugPointer = nil
            shell.songDrag.end()
            TableCheck.table(window)?.controller?.debugEndDrag()
        }
    }

    private static func sourceSongs(_ source: String, shell: ShellModel, window: NSWindow) -> [SongRow] {
        if source.hasPrefix("table:"), let c = TableCheck.table(window)?.controller {
            return source.dropFirst(6).split(separator: "+").compactMap { Int($0) }.compactMap { c.songs?.song(at: $0) }
        }
        if source.hasPrefix("ids:"), let library = shell.library as? CoreLibraryModel {
            return source.dropFirst(4).split(separator: "+").compactMap { library.core?.songById(id: String($0)).map { SongRow(core: $0) } }
        }
        return []
    }

    /// The deepest view under the point that takes dragged songs. SwiftUI's drop targets register for `public.data`,
    /// which `com.davekai.Navia.songs` conforms to.
    private static func destination(at point: NSPoint, in window: NSWindow) -> NSView? {
        let takes: Set<NSPasteboard.PasteboardType> = [.naviaSongs, NSPasteboard.PasteboardType(UTType.data.identifier)]
        func search(_ view: NSView) -> NSView? {
            guard !view.isHidden, view.alphaValue > 0 else { return nil }
            let local = view.convert(point, from: nil)
            let inside = view.bounds.contains(local)
            // A SwiftUI drop target can be a zero-size marker, so its siblings still need searching.
            for child in view.subviews.reversed() { if let found = search(child) { return found } }
            return inside && !takes.isDisjoint(with: view.registeredDraggedTypes) ? view : nil
        }
        return window.contentView?.superview.flatMap(search)
    }

    private static func report(_ shell: ShellModel, value: String, dest: NSView) -> [String: Any] {
        let drag = QueueDragDebug.current
        var out: [String: Any] = ["songDrop": value, "dest": String(describing: type(of: dest)), "active": shell.songDrag.active,
                                  "over": shell.songDrag.over ?? "", "refused": shell.songDrag.refused, "toast": shell.toast ?? ""]
        if let d = shell.duplicates { out["dialog"] = ["title": L("Already in {name}", ["name": d.target.name]), "text": DuplicatesLayer.text(d), "songs": d.songs.count, "fresh": d.fresh.count] }
        if let drag {
            out["queueGap"] = ["incoming": drag.incoming ?? 0, "before": drag.before ?? -1, "intoFirst": drag.intoFirst,
                               "slotY": Double(drag.slot?.minY ?? -1), "slotH": Double(drag.slot?.height ?? 0), "splitShift": Double(drag.splitShift),
                               "shifts": (0..<min(drag.count, 8)).map { Double(drag.shift($0)) },
                               "follow": Double(drag.followShift), "drawnSlot": VisualStats.dragSlot]
        }
        let player = shell.player
        out["queue"] = ["source": player.queueSource?.title ?? "", "queued": player.queuedCount,
                        "upNext": (0..<min(player.upcomingCount, 10)).compactMap { player.upcoming(at: $0).map { ($0.queued ? "[q] " : "") + $0.song.title } }]
        return out
    }

    /// `songDragCard=[SOURCE|]FILE`: writes the drag image for SOURCE's songs (default the table's first two) as a PNG,
    /// with whether the lead cover was cached.
    static func card(_ value: String, shell: ShellModel, window: NSWindow) -> [String: Any] {
        let parts = value.split(separator: "|").map(String.init)
        guard parts.count == 2, let view = window.contentView else { return ["songDragCard": "bad"] }
        let songs = sourceSongs(parts[0], shell: shell, window: window)
        let cached = songs.first?.artworkURL.map { ArtworkLoader.shared.cached($0, pixels: Int(T.thumb * 2)) != nil } ?? false
        guard let lead = songs.first, let card = SongDragCard.dragImage(lead: lead, count: songs.count, at: .zero, in: view, theme: shell.theme),
              let tiff = card.image.tiffRepresentation, let png = NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:]) else { return ["songDragCard": "none"] }
        try? png.write(to: URL(fileURLWithPath: parts[1]))
        return ["songDragCard": value, "songs": songs.count, "lead": lead.title, "coverCached": cached, "size": [Double(card.image.size.width), Double(card.image.size.height)]]
    }

    private static func outsideTask(_ work: @escaping @MainActor () -> Void) {
        DispatchQueue.main.async { MainActor.assumeIsolated { work() } }
    }
}
#endif
