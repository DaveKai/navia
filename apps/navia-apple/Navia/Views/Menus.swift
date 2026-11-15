import AppKit
import SwiftUI

/// One row of a menu. Menus are native, like the app's window controls.
struct MenuEntry {
    var title = ""
    var checked = false
    var enabled = true
    /// Why a disabled row can't be used, shown as its tooltip.
    var reason: String? = nil
    var isDivider = false
    /// A submenu in place of an action. The row's action isn't called.
    var submenu: [MenuEntry]? = nil
    /// With a submenu, the placeholder of a filter field at its top. Typing narrows the submenu's `filterable` rows.
    var find: String? = nil
    /// With `find`: shown in place of the rows when the filter matches nothing.
    var empty: String? = nil
    /// A submenu row the filter narrows (the playlists, not New Playlist).
    var filterable = false
    /// An SF Symbol for the row. Without one, `MenuIcons` looks the title up, so an action has the same icon in every
    /// menu.
    var symbol: String? = nil
    /// A key equivalent shown at the row's end, such as "," for Settings. Command unless `shortcutModifiers` says otherwise.
    var shortcut: String? = nil
    var shortcutModifiers: NSEvent.ModifierFlags = .command
    /// Drawn as a switch that flips without closing the menu (needs `checked`).
    var isSwitch = false
    /// A custom row in place of a title, such as who is signed in.
    var view: NSView? = nil
    /// A section heading, drawn as the system's section header.
    var isHeader = false
    /// A second, dimmer line under the title (a receiver's model, what it's doing, or why it can't be used).
    var subtitle: String? = nil
    var action: () -> Void = {}

    static let divider = MenuEntry(isDivider: true)
}

/// Menu icons as SF Symbols, keyed by the row's English text. Titles are matched after translation.
enum MenuIcons {
    @MainActor static func symbol(for title: String) -> String? { table[title] }

    @MainActor private static var table: [String: String] {
        if let cached, cachedLanguage == currentLanguage() { return cached }
        let pairs: [(String, String)] = [
            ("Play Next", "arrow.turn.up.right"), ("Add to Queue", "text.line.first.and.arrowtriangle.forward"),
            ("Add to Playlist", "text.badge.plus"), ("Remove Download", "xmark"), ("Cancel Download", "xmark"),
            ("Remove Downloads", "xmark"), ("Download Selected", "arrow.down"),
            ("Go to Album", "opticaldisc"), ("Go to Artist", "music.microphone"), ("Start Song Radio", "radio"),
            ("Start Artist Radio", "radio"), ("Share…", "square.and.arrow.up"), ("Stop Station", "stop"),
            ("Private Mode", "eye.slash"), ("Mini Player", "pip"), ("Settings…", "gearshape"), ("Change Server", "server.rack"),
            ("Sign Out", "rectangle.portrait.and.arrow.right"), ("Public on this server", "person.2"),
            ("Create Share Link", "link"), ("Edit Details", "pencil"), ("Edit Rules", "bolt"),
            ("Album Collage", "photo"), ("Generated Cover", "sparkle"), ("Upload Image", "square.and.arrow.up.on.square"),
            ("Remove Image", "trash"), ("Export as M3U", "square.and.arrow.up.on.square"),
            ("New Playlist", "music.note.list"), ("New Smart Playlist", "bolt"), ("Remove from This Playlist", "minus.circle"),
        ]
        let built = Dictionary(pairs.map { (L($0.0), $0.1) }, uniquingKeysWith: { a, _ in a })
        cached = built
        cachedLanguage = currentLanguage()
        return built
    }

    @MainActor private static var cached: [String: String]?
    @MainActor private static var cachedLanguage = ""

    /// Every icon sits centered in a cell of this size, so titles line up whatever the symbol's width.
    static let cell = NSSize(width: 20, height: 16)

    /// The icon at menu size, centered in `cell`, in the row's text color. A template image in a title attachment isn't
    /// tinted by the menu, so the color goes on the symbol itself. `nil` gives an empty cell.
    @MainActor static func image(_ symbol: String?, dimmed: Bool = false) -> NSImage {
        let color: NSColor = dimmed ? .disabledControlTextColor : .labelColor
        let config = NSImage.SymbolConfiguration(pointSize: 14, weight: .regular).applying(NSImage.SymbolConfiguration(paletteColors: [color]))
        let glyph = symbol.flatMap { NSImage(systemSymbolName: $0, accessibilityDescription: nil)?.withSymbolConfiguration(config) }
        let image = NSImage(size: cell, flipped: false) { rect in
            guard let glyph else { return true }
            let s = glyph.size
            glyph.draw(in: NSRect(x: rect.midX - s.width / 2, y: rect.midY - s.height / 2, width: s.width, height: s.height))
            return true
        }
        image.isTemplate = false
        return image
    }

    /// The row's title led by its icon. macOS 27 doesn't draw `NSMenuItem.image` in these menus, but an image in the
    /// attributed title draws and takes the row's text color.
    @MainActor static func title(_ text: String, symbol: String?, enabled: Bool) -> NSAttributedString {
        let font = NSFont.menuFont(ofSize: 0)
        let attachment = NSTextAttachment()
        attachment.image = image(symbol, dimmed: !enabled)
        attachment.bounds = NSRect(x: 0, y: (font.capHeight - cell.height) / 2, width: cell.width, height: cell.height)
        let out = NSMutableAttributedString(attachment: attachment)
        var attributes: [NSAttributedString.Key: Any] = [.font: font]
        if !enabled { attributes[.foregroundColor] = NSColor.disabledControlTextColor }
        out.addAttributes(attributes, range: NSRange(location: 0, length: out.length))
        out.append(NSAttributedString(string: "  " + text, attributes: attributes))
        return out
    }
}

private final class MenuHandler: NSObject {
    let run: () -> Void
    init(_ run: @escaping () -> Void) { self.run = run }
    @objc func fire() { run() }
}

/// A right-click menu for one thing outside a list, as `.rightClickMenu { entries }`. Only right-clicks and
/// Control-clicks reach it. Other clicks pass through.
struct RightClickMenu: NSViewRepresentable {
    let entries: () -> [MenuEntry]

    func makeNSView(context: Context) -> RightClickMenuView { RightClickMenuView() }
    func updateNSView(_ view: RightClickMenuView, context: Context) { view.entries = entries }
}

final class RightClickMenuView: NSView {
    var entries: () -> [MenuEntry] = { [] }

    override func hitTest(_ point: NSPoint) -> NSView? {
        guard let event = NSApp.currentEvent else { return nil }
        let menuClick = event.type == .rightMouseDown || (event.type == .leftMouseDown && event.modifierFlags.contains(.control))
        return menuClick ? super.hitTest(point) : nil
    }

    override func menu(for event: NSEvent) -> NSMenu? {
        let items = entries()
        return items.isEmpty ? nil : makeMenu(items)
    }
}

extension View {
    func rightClickMenu(_ entries: @escaping () -> [MenuEntry]) -> some View { overlay(RightClickMenu(entries: entries)) }
}

func makeMenu(_ entries: [MenuEntry], find: String? = nil, empty: String? = nil) -> NSMenu {
    let menu = NSMenu()
    menu.autoenablesItems = false
    var filter: MenuFindField?
    if let find {
        let field = MenuFindField(placeholder: find)
        let item = NSMenuItem()
        item.view = field
        menu.addItem(item)
        menu.delegate = field
        filter = field
    }
    let symbols = entries.map { $0.isDivider || $0.isHeader || $0.view != nil ? nil : ($0.symbol ?? MenuIcons.symbol(for: $0.title)) }
    let anyIcon = symbols.contains { $0 != nil }
    for (index, entry) in entries.enumerated() {
        if entry.isDivider { menu.addItem(.separator()); continue }
        if entry.isHeader { menu.addItem(.sectionHeader(title: entry.title)); continue }
        if let view = entry.view {
            let item = NSMenuItem()
            item.view = view
            menu.addItem(item)
            continue
        }
        let item = NSMenuItem(title: entry.title, action: #selector(MenuHandler.fire), keyEquivalent: entry.shortcut ?? "")
        if entry.shortcut != nil { item.keyEquivalentModifierMask = entry.shortcutModifiers }
        if anyIcon { item.attributedTitle = MenuIcons.title(entry.title, symbol: symbols[index], enabled: entry.enabled) }
        if entry.isSwitch {
            item.view = MenuSwitchRow(title: entry.title, image: MenuIcons.image(symbols[index]), on: entry.checked, action: entry.action)
            menu.addItem(item)
            continue
        }
        if let submenu = entry.submenu {
            item.action = nil
            item.submenu = makeMenu(submenu, find: entry.find, empty: entry.empty)
        } else {
            let target = MenuHandler(entry.action)
            item.target = target
            item.representedObject = target // NSMenuItem holds its target weakly.
        }
        item.isEnabled = entry.enabled
        if !entry.enabled, let reason = entry.reason { item.toolTip = reason }
        if let subtitle = entry.subtitle { item.subtitle = subtitle }
        item.state = entry.checked ? .on : .off
        if entry.filterable { filter?.filterable.append(item) }
        menu.addItem(item)
    }
    if let filter, let empty {
        let item = NSMenuItem(title: empty, action: nil, keyEquivalent: "")
        item.isEnabled = false
        item.isHidden = true
        filter.emptyItem = item
        menu.addItem(item)
    }
    return menu
}

/// The filter field at the top of a submenu. It takes the keyboard when the submenu opens, and Return picks the
/// first match.
final class MenuFindField: NSView, NSMenuDelegate, NSSearchFieldDelegate {
    let field = NSSearchField()
    var filterable: [NSMenuItem] = []
    var emptyItem: NSMenuItem?

    init(placeholder: String) {
        super.init(frame: NSRect(x: 0, y: 0, width: T.submenuW, height: T.px(34) + T.px(8)))
        field.placeholderString = placeholder
        field.focusRingType = .none
        field.sendsSearchStringImmediately = true
        field.delegate = self
        field.translatesAutoresizingMaskIntoConstraints = false
        addSubview(field)
        NSLayoutConstraint.activate([
            field.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 14),
            field.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -14),
            field.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    func menuWillOpen(_ menu: NSMenu) {
        field.stringValue = ""
        apply()
        // The menu's window exists only once it's tracking, so focus the field then.
        RunLoop.main.perform(inModes: [.eventTracking, .default]) { [weak self] in
            guard let self else { return }
            self.window?.makeFirstResponder(self.field)
        }
    }

    func controlTextDidChange(_ obj: Notification) { apply() }

    /// Return in the field: the first playlist that matches.
    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        guard selector == #selector(NSResponder.insertNewline(_:)), !field.stringValue.isEmpty,
              let first = filterable.first(where: { !$0.isHidden }), let menu = first.menu else { return false }
        menu.cancelTracking()
        if let index = menu.items.firstIndex(of: first) { menu.performActionForItem(at: index) }
        return true
    }

    private func apply() {
        let needle = field.stringValue.trimmingCharacters(in: .whitespaces).lowercased()
        var any = false
        for item in filterable {
            let title = item.attributedTitle?.string ?? item.title
            item.isHidden = !needle.isEmpty && !title.lowercased().contains(needle)
            any = any || !item.isHidden
        }
        emptyItem?.isHidden = any || filterable.isEmpty && needle.isEmpty
    }

    #if DEBUG
    /// For checks: types into the filter as the keyboard would.
    func debugType(_ text: String) { field.stringValue = text; apply() }
    #endif
}

/// The song row menu.
enum SongMenu {
    /// Get Info, last in a song's, album's, playlist's or artist's menu, with ⌘I.
    static func info(_ target: InfoTarget, shell: ShellModel) -> MenuEntry {
        MenuEntry(title: L("Get Info"), symbol: "info.circle", shortcut: "i") { shell.openInfo(target) }
    }

    /// Add to Playlist: a submenu with a filter, New Playlist (named `name` to start), then the user's own playlists,
    /// most recently changed first.
    static func addToPlaylist(_ songs: [SongRow], name: String, title: String = L("Add to Playlist"), shell: ShellModel) -> MenuEntry {
        guard !songs.isEmpty else { return MenuEntry(title: title, enabled: false, reason: L("No songs to add"), symbol: "text.badge.plus") }
        var rows = [MenuEntry(title: L("New Playlist"), symbol: "plus") { shell.openNewPlaylist(songs: songs, name: name) }]
        rows += shell.library.playlistTargets().map { target in
            MenuEntry(title: target.name, filterable: true) { shell.addSongs(songs, toPlaylist: target) }
        }
        return MenuEntry(title: title, submenu: rows, find: L("Find a playlist"), empty: L("No playlists match"), symbol: "text.badge.plus")
    }

    /// Remove from This Playlist, for rows of a playlist the user can edit.
    static func remove(count: Int, action: @escaping () -> Void) -> MenuEntry {
        MenuEntry(title: count > 1 ? L("Remove {n} from This Playlist", ["n": fmtInt(count)]) : L("Remove from This Playlist"), symbol: "minus.circle", action: action)
    }

    /// Remove from Queue. The X and Delete do the same.
    static func removeFromQueue(count: Int, action: @escaping () -> Void) -> MenuEntry {
        MenuEntry(title: count > 1 ? L("Remove {n} from Queue", ["n": fmtInt(count)]) : L("Remove from Queue"), symbol: "minus.circle", action: action)
    }

    static func single(_ song: SongRow, shell: ShellModel, remove: MenuEntry? = nil) -> [MenuEntry] {
        let library = shell.library
        var entries = [
            MenuEntry(title: L("Play Next")) { shell.player.playNext(song) },
            MenuEntry(title: L("Add to Queue")) { shell.player.addToQueue(song) },
            addToPlaylist([song], name: song.title, shell: shell),
        ]
        if let remove { entries.append(remove) }
        if library.isDownloaded(song: song) {
            entries.append(MenuEntry(title: L("Remove Download")) { library.setDownloaded(songs: [song], false) })
        } else if library.isDownloading(song: song) {
            entries.append(MenuEntry(title: L("Cancel Download")) { library.setDownloaded(songs: [song], false) })
        }
        entries.append(.divider)
        entries.append(MenuEntry(title: L("Go to Album")) { shell.goToAlbum(of: song) })
        entries.append(artistEntry(song, shell: shell))
        entries.append(MenuEntry(title: L("Start Song Radio"), enabled: !library.knownNoRadio(song)) { shell.startSongRadio(song) })
        entries.append(shell.shareEntry(songs: [song]))
        entries.append(info(.song(song.id), shell: shell))
        return entries
    }

    /// One artist goes straight there. A song with several credited artists lists them, in the song's own order.
    private static func artistEntry(_ song: SongRow, shell: ShellModel) -> MenuEntry {
        let targets = shell.artistTargets(of: song)
        guard targets.count > 1 else {
            return MenuEntry(title: L("Go to Artist"), enabled: !targets.isEmpty) { if let target = targets.first { shell.goToArtist(target) } }
        }
        return MenuEntry(title: L("Go to Artist"), submenu: targets.map { target in
            MenuEntry(title: target.name) { shell.goToArtist(target) }
        })
    }

    static func several(_ songs: [SongRow], shell: ShellModel, remove: MenuEntry? = nil) -> [MenuEntry] {
        let library = shell.library
        let downloaded = songs.allSatisfy { library.isDownloaded(song: $0) }
        return [
            MenuEntry(title: L("Play {n} Next", ["n": fmtInt(songs.count)]), symbol: "text.line.first.and.arrowtriangle.forward") { songs.reversed().forEach { shell.player.playNext($0) } },
            MenuEntry(title: L("Add {n} to Queue", ["n": fmtInt(songs.count)]), symbol: "text.line.first.and.arrowtriangle.forward") { songs.forEach { shell.player.addToQueue($0) } },
            addToPlaylist(songs, name: "", title: L("Add {n} to Playlist", ["n": fmtInt(songs.count)]), shell: shell),
        ] + (remove.map { [$0] } ?? []) + [
            shell.shareEntry(songs: songs),
            downloaded
                ? MenuEntry(title: L("Remove Downloads")) { library.setDownloaded(songs: songs, false) }
                : MenuEntry(title: L("Download Selected")) { library.setDownloaded(songs: songs, true) },
        ]
    }
}

/// A button that opens a native menu. Sits over any label, so the button keeps its own look.
struct MenuHitArea: NSViewRepresentable {
    /// Lines the menu's right edge up with the button's, for buttons at the window's right.
    var trailing = false
    let entries: () -> [MenuEntry]

    func makeNSView(context: Context) -> MenuOpeningView {
        let view = MenuOpeningView()
        view.entries = entries
        view.trailing = trailing
        return view
    }

    func updateNSView(_ nsView: MenuOpeningView, context: Context) { nsView.entries = entries; nsView.trailing = trailing }
}

final class MenuOpeningView: NSView {
    var entries: () -> [MenuEntry] = { [] }
    var trailing = false
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
    override func mouseDown(with event: NSEvent) { open() }
    /// Opens the menu below the button, or above it when it fits better there, so macOS doesn't clip it with a scroll
    /// arrow. Kept on screen sideways.
    func open() {
        let items = entries()
        guard !items.isEmpty else { return }
        let menu = makeMenu(items)
        MenuOpeningView.lastOpened = menu
        menu.popUp(positioning: nil, at: placement(for: menu.size).point, in: self)
    }

    /// Where a menu of `size` opens: its top-left corner in this view's coordinates, and whether that's above the button.
    func placement(for size: NSSize) -> (point: NSPoint, above: Bool) {
        let gap: CGFloat = 4
        var left = trailing ? bounds.maxX - size.width : 0
        var above = false
        if let window, let visible = window.screen?.visibleFrame {
            let button = window.convertToScreen(convert(bounds, to: nil))
            let below = button.minY - gap - visible.minY
            let over = visible.maxY - (button.maxY + gap)
            above = size.height > below && over > below
            left = min(max(button.minX + left, visible.minX), visible.maxX - size.width) - button.minX
        }
        let top: CGFloat = isFlipped ? (above ? bounds.minY - gap - size.height : bounds.maxY + gap) : (above ? bounds.maxY + gap + size.height : bounds.minY - gap)
        return (NSPoint(x: left, y: top), above)
    }

    /// The menu this view opened last, for checks.
    nonisolated(unsafe) static weak var lastOpened: NSMenu?
}

/// An icon button that opens a menu.
struct MenuIconButton: View {
    let symbol: String
    let label: String
    var size: CGFloat = T.px(17)
    var width: CGFloat = T.iconW
    var height: CGFloat = T.iconH
    var tint: Color? = nil
    /// Set when nothing in the menu can be used yet: the button dims like a disabled one and says why.
    var disabledReason: String? = nil
    let entries: () -> [MenuEntry]
    @State private var hovering = false
    @Environment(\.isSnapshot) private var isSnapshot

    var body: some View {
        let disabled = disabledReason != nil
        Image(systemName: symbol)
            .font(.system(size: size))
            .foregroundStyle(tint.map { AnyShapeStyle($0) } ?? AnyShapeStyle(.foreground))
            .frame(width: width, height: height)
            .modifier(HoverBrighten(on: !disabled && (hovering || HoverDebug.forced(label))))
            .opacity(disabled ? 0.3 : (hovering ? 1 : 0.88))
            .overlay { if !isSnapshot && !disabled { MenuHitArea(entries: entries) } }
            .contentShape(Capsule())
            .onHover { hovering = $0 }
            .help(disabledReason.map { "\(label): \($0)" } ?? label)
            .accessibilityLabel(label)
            .accessibilityAddTraits(.isButton)
    }
}

/// A text pill that opens a menu, for the Search page's scope picker.
struct MenuTextButton: View {
    let title: String
    let entries: () -> [MenuEntry]
    @State private var hovering = false
    @Environment(\.isSnapshot) private var isSnapshot

    var body: some View {
        HStack(spacing: T.px(6)) {
            Text(title).font(.navia(T.textMD, .medium))
            Image(systemName: "chevron.up.chevron.down").font(.system(size: T.px(11), weight: .bold))
        }
        .padding(.horizontal, T.px(12))
        .frame(height: T.iconH)
        .background(HoverFillBackground(on: hovering || HoverDebug.forced(title)))
        .contentShape(Capsule())
        .overlay { if !isSnapshot { MenuHitArea(entries: entries) } }
        .onHover { hovering = $0 }
    }
}

/// A menu row with a switch at its end, such as Private Mode. A view-based item doesn't end the menu's tracking,
/// so the menu stays open when it flips.
final class MenuSwitchRow: NSView {
    private let toggle = NSSwitch()
    private let action: () -> Void

    init(title: String, image: NSImage?, on: Bool, action: @escaping () -> Void) {
        self.action = action
        super.init(frame: NSRect(x: 0, y: 0, width: 240, height: 28))
        // Follow the menu's width, so the switch ends at the same inset as the other rows.
        autoresizingMask = [.width]
        let icon = NSImageView()
        icon.image = image
        icon.contentTintColor = .labelColor
        let label = NSTextField(labelWithString: title)
        label.font = .menuFont(ofSize: 0)
        toggle.controlSize = .mini
        toggle.state = on ? .on : .off
        toggle.target = self
        toggle.action = #selector(flipped)
        for view in [icon, label, toggle] as [NSView] {
            view.translatesAutoresizingMaskIntoConstraints = false
            addSubview(view)
        }
        NSLayoutConstraint.activate([
            icon.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 14),
            icon.centerYAnchor.constraint(equalTo: centerYAnchor),
            label.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 6),
            label.centerYAnchor.constraint(equalTo: centerYAnchor),
            toggle.leadingAnchor.constraint(greaterThanOrEqualTo: label.trailingAnchor, constant: 18),
            toggle.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -14),
            toggle.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
        setAccessibilityElement(false)
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    /// A click anywhere on the row flips the switch.
    override func mouseUp(with event: NSEvent) {
        toggle.state = toggle.state == .on ? .off : .on
        flipped()
    }

    @objc private func flipped() { action() }

    #if DEBUG
    /// For checks: flips it as a click on the row would.
    func debugFlip() { mouseUp(with: NSEvent()) }
    var debugOn: Bool { toggle.state == .on }
    /// For checks: the switch's inset from the row's right edge, and the row's width.
    var debugInsets: (right: CGFloat, width: CGFloat) { (bounds.maxX - toggle.frame.maxX, bounds.width) }
    #endif
}
