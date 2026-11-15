import AppKit
import SwiftUI

/// Colors the AppKit table needs, refreshed from SwiftUI whenever they change.
enum SongListVariant { case full, album }

struct TableColors: Equatable {
    var accent: RGBA
    var accentSoft: RGBA
    var onAccent: RGBA
    /// A selected row: the accent at 30%.
    var rowSelected: RGBA { accent.opacity(Theme.rowSelectedAlpha) }
}

/// Columns a listener can show or hide from a song table's header. Song Name and Time always show.
enum SongColumn: String, CaseIterable, Sendable {
    case artist, album, genre, added, plays

    var label: String {
        switch self {
        case .artist: L("Artist")
        case .album: L("Album")
        case .genre: L("Genre")
        case .added: L("Date Added")
        case .plays: L("Plays")
        }
    }

    /// Its header's position (0 Song Name and 1 Time come first).
    var header: Int { 2 + (SongColumn.allCases.firstIndex(of: self) ?? 0) }

    /// What the listener chose, per account. Genre, Date Added and Plays start hidden. An older Genre setting carries
    /// over.
    static var shown: Set<SongColumn> {
        if let saved = UserDefaults.standard.array(forKey: AccountPrefs.key("songColumns")) as? [String] {
            return Set(saved.compactMap(SongColumn.init(rawValue:)))
        }
        return UserDefaults.standard.bool(forKey: "showGenreColumn") ? [.artist, .album, .genre] : [.artist, .album]
    }

    static func toggle(_ column: SongColumn) {
        var set = shown
        if set.remove(column) == nil { set.insert(column) }
        UserDefaults.standard.set(allCases.filter(set.contains).map(\.rawValue), forKey: AccountPrefs.key("songColumns"))
        NotificationCenter.default.post(name: .naviaSongColumnsChanged, object: nil)
    }
}

extension Notification.Name {
    nonisolated static let naviaSongColumnsChanged = Notification.Name("naviaSongColumnsChanged")
}

/// The song table's grid columns and their width breakpoints: narrow tables drop Genre, Date Added and Plays, then
/// Album, then Artist.
struct SongColumns {
    let lead: CGFloat
    let title: CGFloat
    let time: CGFloat
    let artist: CGFloat
    let album: CGFloat
    let genre: CGFloat
    let added: CGFloat
    let plays: CGFloat
    let actions: CGFloat
    var showsArtist: Bool { artist > 0 }
    var showsAlbum: Bool { album > 0 }
    var showsGenre: Bool { genre > 0 }

    /// `width` is the row's width inside the gutters.
    init(width: CGFloat, compact: Bool, variant: SongListVariant = .full, shown: Set<SongColumn> = SongColumn.shown) {
        let cssWidth = width / T.scale
        let leftPad: CGFloat = 5
        if variant == .album {
            added = 0; plays = 0
            let lead: CGFloat = cssWidth <= 500 ? 40 : 55
            let time: CGFloat = cssWidth <= 500 ? 58 : 78
            let actions: CGFloat = cssWidth <= 500 ? 124 : 132
            let room = max(0, cssWidth - leftPad - lead - time - actions)
            self.lead = T.px(leftPad + lead)
            title = T.px(min(305, room))
            self.time = T.px(time)
            artist = 0; album = 0; genre = 0
            self.actions = T.px(actions)
            return
        }
        let lead: CGFloat = compact ? 24 : 55
        let timeCSS: CGFloat, actionsCSS: CGFloat
        var fr: (title: CGFloat, artist: CGFloat, album: CGFloat, genre: CGFloat)
        var fixed: (added: CGFloat, plays: CGFloat) = (0, 0)
        switch cssWidth {
        case ...500: (timeCSS, actionsCSS, fr) = (58, 124, (1, 0, 0, 0))
        case ...700: (timeCSS, actionsCSS, fr) = (62, 124, (1, 1, 0, 0))
        case ...900: (timeCSS, actionsCSS, fr) = (70, 132, (2, 1.2, 1.4, 0))
        default:
            (timeCSS, actionsCSS, fr) = (78, 132, (298, 172, 228, 95))
            fixed = (shown.contains(.added) ? 122 : 0, shown.contains(.plays) ? 76 : 0)
        }
        if !shown.contains(.artist) { fr.artist = 0 }
        if !shown.contains(.album) { fr.album = 0 }
        if !shown.contains(.genre) { fr.genre = 0 }
        let parts: CGFloat = fr.title + fr.artist + fr.album + fr.genre
        let unit: CGFloat = max(0, cssWidth - leftPad - lead - timeCSS - actionsCSS - fixed.added - fixed.plays) / parts
        self.lead = T.px(leftPad + lead)
        title = T.px(unit * fr.title)
        time = T.px(timeCSS)
        artist = T.px(unit * fr.artist)
        album = T.px(unit * fr.album)
        genre = T.px(unit * fr.genre)
        added = T.px(fixed.added)
        plays = T.px(fixed.plays)
        actions = T.px(actionsCSS)
    }
}

/// An AppKit table, so a whole library scrolls smoothly with reused row views.
/// The headers are real rows: row 0 is the page header's room (or the space under the top bar), row 1 the column
/// header when shown, then the songs, then the room under the bar or dock. No content insets, so scrolling, clicks
/// and hover stay plain AppKit.
struct SongsTable: NSViewRepresentable {
    let songs: any SongsViewModel
    let shell: ShellModel
    let revision: Int
    let editRevision: Int
    let nowPlayingID: String?
    var variant: SongListVariant = .full
    var compact = false
    var pageHeader: AnyView? = nil
    /// Rows stay beside the sidebar and dock, but the scroll view reaches under them so tints do too.
    var leadingInset: CGFloat = 0
    var trailingInset: CGFloat = 0
    var bottomInset: CGFloat = T.px(48)
    /// The list's sort, so its column header is emphasized. Orders with no column emphasize none.
    var sort: SongSort? = nil
    /// Read so a header click that only flips the direction redraws the arrow.
    var descending = false
    /// Press-and-hold reordering: called with song indexes and the index they land before. Nil where songs can't move.
    var reorder: (([Int], Int) -> Void)? = nil
    /// On an editable playlist, removes these songs (song indexes, as shown). Adds Remove from This Playlist to the
    /// row menus.
    var remove: (([Int]) -> Void)? = nil
    /// Disc names by number on albums with more than one disc. A header goes above each disc's first song. Nil
    /// otherwise.
    var discs: [Int: String]? = nil
    /// Songs after the list, in the same scroll (a playlist's Recommended).
    var trailing: TrailingSongs? = nil
    /// Called when the end of the list scrolls near view, so what follows it can be asked for only then.
    var nearEnd: (() -> Void)? = nil

    func makeCoordinator() -> SongsTableController { SongsTableController() }

    func makeNSView(context: Context) -> NSScrollView { context.coordinator.scrollView }

    func updateNSView(_ nsView: NSScrollView, context: Context) {
        context.coordinator.update(songs: songs, shell: shell, revision: revision, editRevision: editRevision, nowPlayingID: nowPlayingID,
                                   colors: TableColors(accent: shell.theme.accent, accentSoft: shell.theme.accentSoft, onAccent: shell.theme.onAccent),
                                   variant: variant, compact: compact, offline: shell.library.isOffline, pageHeader: pageHeader,
                                   leadingInset: leadingInset, trailingInset: trailingInset, bottomInset: bottomInset, discs: discs)
        context.coordinator.setSort(sort, descending: descending)
        context.coordinator.setReorder(reorder)
        context.coordinator.remove = remove
        context.coordinator.nearEnd = nearEnd
        context.coordinator.setTrailing(trailing)
    }
}

/// Songs shown after the list in the same scroll (a playlist's Recommended): a hosted heading, then rows with Add in
/// place of Download and Favorite. They don't select, reorder or drag.
struct TrailingSongs {
    var heading: AnyView
    var songs: [SongRow]
    /// Bumped when the songs change, so the rows reload.
    var revision: Int
    /// The rows dim while the next batch is on its way.
    var dimmed: Bool
    var play: (Int) -> Void
    var add: (SongRow) -> Void
    /// Show the heading even with no songs, for a closing section with no rows of its own.
    var keepsHeading = false

    /// A hosted section after the list and nothing else.
    static func section(_ view: AnyView) -> TrailingSongs {
        TrailingSongs(heading: view, songs: [], revision: 0, dimmed: false, play: { _ in }, add: { _ in }, keepsHeading: true)
    }
}

extension NSPasteboard.PasteboardType {
    /// A song row being moved within one table: its song index.
    static let naviaSongRow = NSPasteboard.PasteboardType("com.davekai.Navia.song-row")
}

final class SongsTableController: NSObject, NSTableViewDataSource, NSTableViewDelegate {
    let scrollView = NSScrollView()
    let table = SongsNSTableView()
    private let header = SongHeaderView()
    private var pageHost: NSHostingView<AnyView>?

    private(set) var songs: (any SongsViewModel)?
    private(set) var shell: ShellModel?
    private(set) var colors = TableColors(accent: Theme.defaultAccent, accentSoft: Theme.defaultAccent, onAccent: RGBA(0, 0, 0))
    private(set) var compact = false
    private(set) var variant: SongListVariant = .full
    private(set) var leadingInset: CGFloat = 0
    private(set) var trailingInset: CGFloat = 0
    /// Offline, rows for songs not on this device are dimmed.
    private(set) var offline = false
    private var nowPlayingID: String?
    private var lastRevision = -1
    private var lastEdit = -1
    private var bottomInset: CGFloat = 0
    private var pageHeaderHeight: CGFloat = 0
    private var didScrollToTop = false
    private var didLayout = false
    private var clickAway: Any?

    private static let cellID = NSUserInterfaceItemIdentifier("song")
    private static let rowID = NSUserInterfaceItemIdentifier("songRow")
    private static let spacerID = NSUserInterfaceItemIdentifier("spacerRow")
    private static let discID = NSUserInterfaceItemIdentifier("disc")

    /// The page tint's height, so the hosted header is tall enough to paint it behind the rows.
    private let tintHeight = T.px(460)

    override init() {
        super.init()
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("main"))
        column.resizingMask = .autoresizingMask
        table.addTableColumn(column)
        table.headerView = nil
        table.style = .plain
        table.intercellSpacing = .zero
        table.rowHeight = aligned(songRowHeight)
        table.backgroundColor = .clear
        table.gridStyleMask = []
        table.usesAlternatingRowBackgroundColors = false
        table.columnAutoresizingStyle = .lastColumnOnlyAutoresizingStyle
        table.focusRingType = .none
        table.allowsEmptySelection = true
        table.allowsMultipleSelection = true
        table.dataSource = self
        table.delegate = self
        table.target = self
        table.doubleAction = #selector(playClicked)
        table.controller = self
        header.controller = self

        scrollView.documentView = table
        scrollView.drawsBackground = false
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = false
        scrollView.autohidesScrollers = true
        scrollView.scrollerStyle = .overlay
        scrollView.scrollerKnobStyle = .light
        scrollView.automaticallyAdjustsContentInsets = false
        scrollView.contentView.postsBoundsChangedNotifications = true
        NotificationCenter.default.addObserver(self, selector: #selector(scrolled), name: NSView.boundsDidChangeNotification, object: scrollView.contentView)
        NotificationCenter.default.addObserver(self, selector: #selector(columnsChanged), name: .naviaSongColumnsChanged, object: nil)

        // Clicking outside the rows clears the selection. Menus run their own event loop, so using one doesn't.
        clickAway = NSEvent.addLocalMonitorForEvents(matching: .leftMouseDown) { [weak self] event in
            guard let self, event.window === self.table.window, let root = event.window?.contentView?.superview else { return event }
            let hit = root.hitTest(root.convert(event.locationInWindow, from: nil))
            let onRows = hit?.isDescendant(of: self.table) == true && hit?.isDescendant(of: self.pageHost ?? NSView()) != true
            if !onRows, self.table.selectedRow >= 0 { self.table.deselectAll(nil) }
            return event
        }
    }

    isolated deinit {
        NotificationCenter.default.removeObserver(self)
        if let clickAway { NSEvent.removeMonitor(clickAway) }
    }

    func update(songs: any SongsViewModel, shell: ShellModel, revision: Int, editRevision: Int, nowPlayingID: String?, colors: TableColors,
                variant: SongListVariant, compact: Bool, offline: Bool, pageHeader: AnyView?, leadingInset: CGFloat, trailingInset: CGFloat, bottomInset: CGFloat,
                discs: [Int: String]? = nil) {
        self.songs = songs
        let discsChanged = discs != self.discs
        self.discs = discs
        self.shell = shell
        let offlineChanged = offline != self.offline
        self.offline = offline
        let compactChanged = compact != self.compact
        let colorsChanged = colors != self.colors
        let nowChanged = nowPlayingID != self.nowPlayingID
        let insetsChanged = leadingInset != self.leadingInset || trailingInset != self.trailingInset
        let variantChanged = variant != self.variant
        self.colors = colors
        self.compact = compact
        self.variant = variant
        self.nowPlayingID = nowPlayingID
        self.leadingInset = leadingInset
        self.trailingInset = trailingInset

        if compactChanged || variantChanged {
            header.variant = variant
            header.compact = compact
            table.rowHeight = aligned(songRowHeight)
        }
        if let pageHeader {
            lastPageHeader = pageHeader
            applyPageHeader()
        }
        let bottomChanged = bottomInset != self.bottomInset
        self.bottomInset = bottomInset
        header.leadingInset = leadingInset
        header.trailingInset = trailingInset
        if compactChanged || insetsChanged || variantChanged { header.needsLayout = true; header.needsDisplay = true }
        if bottomChanged || !didLayout || compactChanged || insetsChanged { applyLayout() }
        if revision != lastRevision || compactChanged || variantChanged || discsChanged {
            lastRevision = revision
            lastEdit = editRevision
            findDiscBreaks()
            table.reloadData()
        } else if colorsChanged || nowChanged || insetsChanged || offlineChanged || editRevision != lastEdit {
            lastEdit = editRevision
            refreshVisibleRows()
        }
    }

    private var lastPageHeader: AnyView?
    private var columnHeaderHeight: CGFloat { compact ? T.rowCompactH : T.rowH }
    private var showsColumnHeader: Bool { variant == .full }

    /// The first row: the page header's measured height, or the room under the top bar on pages without one.
    private var topRowHeight: CGFloat { lastPageHeader != nil ? max(1, pageHeaderHeight) : T.topbarH + T.contentTop }
    /// Everything above the first song.
    private var headersHeight: CGFloat { topRowHeight + (showsColumnHeader ? columnHeaderHeight : 0) }

    // MARK: Rows: headers, songs, the bottom room

    /// Rows before the first song: the top row, and the column header when shown.
    var leadRows: Int { showsColumnHeader ? 2 : 1 }
    private var columnHeaderRow: Int { showsColumnHeader ? 1 : -1 }
    /// The first row after the list's songs and disc headers.
    private var listEnd: Int { leadRows + (songs?.count ?? 0) + discBreaks.count }
    private var bottomRow: Int { listEnd + trailingRows }

    // MARK: Songs after the list

    private var trailing: TrailingSongs?
    var nearEnd: (() -> Void)?
    private var trailingHeadHeight: CGFloat = 0
    private var trailingHeadWidth: CGFloat = 0
    private var trailingRows: Int {
        guard let trailing else { return 0 }
        return trailing.songs.isEmpty ? (trailing.keepsHeading ? 1 : 0) : 1 + trailing.songs.count
    }
    private var trailingHeadRow: Int? { trailingRows > 0 ? listEnd : nil }
    private static let trailingHeadID = NSUserInterfaceItemIdentifier("trailingHead")

    /// The index among the songs after the list for a table row, if the row is one of them.
    func trailingIndex(_ row: Int) -> Int? {
        guard let head = trailingHeadRow, let count = trailing?.songs.count, row > head, row - head - 1 < count else { return nil }
        return row - head - 1
    }

    func trailingSong(_ index: Int) -> SongRow? { trailing.flatMap { $0.songs.indices.contains(index) ? $0.songs[index] : nil } }

    func setTrailing(_ new: TrailingSongs?) {
        let old = trailing
        trailing = new
        measureTrailingHead()
        if old?.revision != new?.revision || (old == nil) != (new == nil) || old?.songs.count != new?.songs.count {
            table.reloadData()
        } else if old?.dimmed != new?.dimmed {
            refreshVisibleRows()
        }
        if let head = trailingHeadRow, let view = table.view(atColumn: 0, row: head, makeIfNecessary: false) as? NSHostingView<AnyView>, let new {
            view.rootView = new.heading
        }
        checkNearEnd()
    }

    private func measureTrailingHead() {
        guard let heading = trailing?.heading else { return }
        let width = max(1, table.bounds.width)
        let probe = NSHostingView(rootView: heading.frame(width: width).fixedSize(horizontal: false, vertical: true))
        let height = probe.fittingSize.height
        trailingHeadWidth = width
        if abs(height - trailingHeadHeight) > 0.5 {
            trailingHeadHeight = height
            if let head = trailingHeadRow { table.noteHeightOfRows(withIndexesChanged: IndexSet(integer: head)) }
        }
    }

    /// Tells the page once the end of the list is within `T.nearViewMargin` of the visible rows.
    private func checkNearEnd() {
        guard let nearEnd, table.numberOfRows > listEnd else { return }
        let end = table.rect(ofRow: min(listEnd, table.numberOfRows - 1)).minY
        // Later, so the page's state never changes while SwiftUI is updating this view.
        if scrollView.contentView.bounds.maxY + T.nearViewMargin >= end { DispatchQueue.main.async { nearEnd() } }
    }

    func trailingMenu(_ index: Int) -> NSMenu? {
        guard let shell, let song = trailingSong(index) else { return nil }
        return makeMenu(SongMenu.single(song, shell: shell, remove: nil))
    }

    func playTrailing(_ index: Int) { trailing?.play(index) }

    func addTrailing(_ index: Int) { trailingSong(index).map { trailing?.add($0) } }

    /// Whether a table row is a song of the list or one after it: the rows that hover and take links and menus.
    func isSongRow(_ row: Int) -> Bool { songIndex(row) != nil || trailingIndex(row) != nil }

    private var discs: [Int: String]?
    /// Disc headers: each goes above the song at `index`. Header k is table row `leadRows + index + k`.
    private(set) var discBreaks: [(index: Int, title: String)] = []

    /// Where each disc starts among the songs shown, on an album with more than one disc.
    private func findDiscBreaks() {
        discBreaks = []
        guard let discs, variant == .album, let songs else { return }
        var last: Int?
        for i in 0..<songs.count {
            guard let disc = songs.song(at: i)?.disc, disc != last else { continue }
            last = disc
            let name = discs[disc]?.trimmingCharacters(in: .whitespaces) ?? ""
            discBreaks.append((i, name.isEmpty ? L("Disc {number}", ["number": fmtInt(disc)]) : name))
        }
    }

    /// The disc header's title at a table row, if that row is one.
    func discTitle(_ row: Int) -> String? {
        let r = row - leadRows
        for (k, b) in discBreaks.enumerated() {
            if b.index + k == r { return b.title }
            if b.index + k > r { break }
        }
        return nil
    }

    /// The song index for a table row, or nil for a header, a disc header or the bottom room.
    func songIndex(_ row: Int) -> Int? {
        let r = row - leadRows
        guard r >= 0 else { return nil }
        var before = 0
        for (k, b) in discBreaks.enumerated() {
            let at = b.index + k
            if at == r { return nil }
            if at > r { break }
            before += 1
        }
        let index = r - before
        return index < (songs?.count ?? 0) ? index : nil
    }

    func tableRow(_ index: Int) -> Int { index + leadRows + discBreaks.prefix { $0.index <= index }.count }

    private func applyLayout() {
        let wasAtTop = !didScrollToTop || scrollView.contentView.bounds.origin.y <= 1
        didLayout = true
        scrollView.contentInsets = NSEdgeInsets()
        scrollView.scrollerInsets = NSEdgeInsets(top: T.topbarH, left: 0, bottom: bottomInset, right: trailingInset)
        table.keyboardMargins = (T.topbarH, bottomInset)
        noteHeaderHeights()
        if wasAtTop {
            scrollView.contentView.scroll(to: .zero)
            scrollView.reflectScrolledClipView(scrollView.contentView)
            didScrollToTop = true
        }
    }

    /// The header and bottom rows' heights changed. Songs keep theirs.
    private func noteHeaderHeights() {
        var rows = IndexSet(0..<leadRows)
        rows.insert(bottomRow)
        table.noteHeightOfRows(withIndexesChanged: rows.filteredIndexSet { $0 < table.numberOfRows })
        layoutPageHost()
    }

    /// The width the header was last measured at. The table stays invisible until it's measured at a real width, so
    /// the first frame never draws the header over the rows.
    private var measuredWidth: CGFloat = 0

    /// Hosts the SwiftUI page header as the bottom layer, so the page tint can run behind the first rows.
    private func applyPageHeader() {
        guard let view = lastPageHeader else { return }
        let width = max(1, table.bounds.width)
        if pageHost == nil {
            let host = NSHostingView(rootView: AnyView(EmptyView()))
            host.sizingOptions = []
            table.addSubview(host, positioned: .below, relativeTo: nil)
            pageHost = host
            scrollView.alphaValue = 0
        }
        // Measured here in the same pass. The geometry callback reports a frame late, so the header would draw over the
        // first rows for a frame.
        if width > 1, abs(width - measuredWidth) > 0.5 {
            let probe = NSHostingView(rootView: view.frame(width: width).fixedSize(horizontal: false, vertical: true))
            let height = probe.fittingSize.height
            measuredWidth = width
            if height > 1, abs(height - pageHeaderHeight) > 0.5 {
                pageHeaderHeight = height
                noteHeaderHeights()
            }
            // No fade. With `animator()` the page drew, went blank for a frame, then faded in.
            if scrollView.alphaValue == 0 { scrollView.alphaValue = 1 }
        }
        let hostHeight = max(headersHeight, tintHeight)
        layoutPageHost()
        pageHost?.rootView = AnyView(
            view.frame(width: width, alignment: .top)
                .onGeometryChange(for: CGFloat.self, of: { $0.size.height }) { [weak self] height in
                    DispatchQueue.main.async { self?.pageHeaderDidResize(height) }
                }
                .frame(width: width, height: hostHeight, alignment: .top)
        )
    }

    private func pageHeaderDidResize(_ height: CGFloat) {
        guard abs(height - pageHeaderHeight) > 0.5 else { return }
        pageHeaderHeight = height
        noteHeaderHeights()
        applyPageHeader()
    }

    /// Places the hosted page header at the top row as the bottom layer. Its row is empty and lets clicks through.
    private func layoutPageHost() {
        pageHost?.frame = NSRect(x: 0, y: 0, width: max(1, table.bounds.width), height: max(headersHeight, tintHeight))
    }

    func tableResized() {
        applyPageHeader()
        if abs(table.bounds.width - trailingHeadWidth) > 0.5 { measureTrailingHead() }
    }

    func refreshVisibleRows() {
        table.enumerateAvailableRowViews { rowView, row in
            if let index = self.trailingIndex(row) {
                (rowView as? SongRowView)?.colors = self.colors
                self.applyTrailingState(rowView as? SongRowView, index: index)
                (rowView.view(atColumn: 0) as? SongCell).map { self.configureTrailing($0, index: index) }
                return
            }
            guard let index = self.songIndex(row) else { return }
            (rowView as? SongRowView)?.colors = self.colors
            self.applyRowState(rowView as? SongRowView, index: index)
            rowView.needsDisplay = true
            (rowView.view(atColumn: 0) as? SongCell).map { self.configure($0, index: index) }
        }
    }

    private func applyRowState(_ view: SongRowView?, index: Int) {
        view?.leadingInset = leadingInset
        view?.trailingInset = trailingInset
        view?.isDownloaded = variant == .full && (songs?.song(at: index)?.isDownloaded ?? false)
        // The disc header below draws its own rule.
        view?.hidesSeparator = discTitle(tableRow(index) + 1) != nil
        // Rows being dragged stay in place, dimmed, while the gap opens where they'll land.
        view?.alphaValue = draggedSongs.contains(index) ? T.dragOutDim : 1
    }

    /// Dims or restores the visible rows being dragged.
    private func refreshDraggedRows() {
        table.enumerateAvailableRowViews { view, row in
            if let index = self.songIndex(row) { self.applyRowState(view as? SongRowView, index: index) }
        }
    }

    private func configureTrailing(_ cell: SongCell, index: Int) {
        guard let song = trailingSong(index) else { cell.clear(row: index); return }
        cell.configure(song: song, row: index, isNow: song.id == nowPlayingID, compact: false, variant: .full,
                       colors: colors, leadingInset: leadingInset, trailingInset: trailingInset, controller: self, trailing: true)
    }

    private func applyTrailingState(_ view: SongRowView?, index: Int) {
        view?.leadingInset = leadingInset
        view?.trailingInset = trailingInset
        view?.isDownloaded = false
        view?.hidesSeparator = false
        view?.alphaValue = trailing?.dimmed == true ? T.disabledOpacity : 1
    }

    private func configure(_ cell: SongCell, index: Int) {
        guard let song = songs?.song(at: index) else { cell.clear(row: index); return }
        cell.configure(song: song, row: index, isNow: song.id == nowPlayingID, compact: compact, variant: variant,
                       colors: colors, leadingInset: leadingInset, trailingInset: trailingInset, controller: self)
    }

    // MARK: Data source and delegate

    func numberOfRows(in tableView: NSTableView) -> Int { bottomRow + 1 }

    /// Heights rounded to device pixels, so every 1 pt rule lands on a pixel. Fractional token heights would leave the
    /// rules soft.
    func tableView(_ tableView: NSTableView, heightOfRow row: Int) -> CGFloat {
        if row == 0 { return aligned(topRowHeight) }
        if row == columnHeaderRow { return aligned(columnHeaderHeight) }
        if row == bottomRow { return aligned(max(1, bottomInset)) }
        if row == trailingHeadRow { return aligned(max(1, trailingHeadHeight)) }
        if trailingIndex(row) != nil { return aligned(T.rowH) }
        if discTitle(row) != nil { return aligned(T.px(52)) }
        return aligned(songRowHeight)
    }

    /// Album tracks are two lines, title over artists, at the compact row height.
    private var songRowHeight: CGFloat { variant == .album ? T.px(65) : compact ? T.rowCompactH : T.rowH }

    /// `rowHeight` is also NSTableView's estimate for rows it hasn't measured, so it must be the same aligned value.
    func aligned(_ h: CGFloat) -> CGFloat {
        let scale = table.window?.backingScaleFactor ?? NSScreen.main?.backingScaleFactor ?? 2
        return max(1, (h * scale).rounded() / scale)
    }

    /// The display scale, and with it every aligned height, may differ after joining a window or changing display.
    func backingChanged() {
        table.rowHeight = aligned(songRowHeight)
        if table.numberOfRows > 0 { table.noteHeightOfRows(withIndexesChanged: IndexSet(0..<table.numberOfRows)) }
    }

    /// Only songs can be selected, so Select All and Shift-click across the headers take songs only.
    func tableView(_ tableView: NSTableView, selectionIndexesForProposedSelection proposed: IndexSet) -> IndexSet {
        proposed.filteredIndexSet { songIndex($0) != nil }
    }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        if row == columnHeaderRow { return header }
        if let title = discTitle(row) {
            let view = (tableView.makeView(withIdentifier: Self.discID, owner: self) as? DiscHeaderView) ?? {
                let new = DiscHeaderView()
                new.identifier = Self.discID
                return new
            }()
            view.configure(title: title, leadingInset: leadingInset, trailingInset: trailingInset)
            return view
        }
        if row == trailingHeadRow, let heading = trailing?.heading {
            let view = (tableView.makeView(withIdentifier: Self.trailingHeadID, owner: self) as? NSHostingView<AnyView>) ?? {
                let new = NSHostingView(rootView: heading)
                new.identifier = Self.trailingHeadID
                new.sizingOptions = []
                return new
            }()
            view.rootView = heading
            return view
        }
        if let index = trailingIndex(row) {
            let cell = (tableView.makeView(withIdentifier: Self.cellID, owner: self) as? SongCell) ?? {
                let new = SongCell()
                new.identifier = Self.cellID
                return new
            }()
            configureTrailing(cell, index: index)
            cell.setHovered(row == table.hoveredRow)
            return cell
        }
        guard let index = songIndex(row) else { return nil }
        let cell = (tableView.makeView(withIdentifier: Self.cellID, owner: self) as? SongCell) ?? {
            let new = SongCell()
            new.identifier = Self.cellID
            return new
        }()
        configure(cell, index: index)
        cell.setHovered(row == table.hoveredRow)
        return cell
    }

    func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
        if let index = trailingIndex(row) {
            let view = (tableView.makeView(withIdentifier: Self.rowID, owner: self) as? SongRowView) ?? {
                let new = SongRowView()
                new.identifier = Self.rowID
                return new
            }()
            view.colors = colors
            view.compact = false
            view.album = false
            applyTrailingState(view, index: index)
            view.isHovered = row == table.hoveredRow
            return view
        }
        guard let index = songIndex(row) else {
            let spacer = (tableView.makeView(withIdentifier: Self.spacerID, owner: self) as? SpacerRowView) ?? {
                let new = SpacerRowView()
                new.identifier = Self.spacerID
                return new
            }()
            return spacer
        }
        let view = (tableView.makeView(withIdentifier: Self.rowID, owner: self) as? SongRowView) ?? {
            let new = SongRowView()
            new.identifier = Self.rowID
            return new
        }()
        view.colors = colors
        view.compact = compact
        view.album = variant == .album
        applyRowState(view, index: index)
        view.isHovered = row == table.hoveredRow
        return view
    }

    // MARK: Actions

    @objc func playClicked() {
        let flags = NSApp.currentEvent?.modifierFlags.intersection([.command, .control, .shift]) ?? []
        if flags.isEmpty, table.clickedRow >= 0, let index = trailingIndex(table.clickedRow) { playTrailing(index); return }
        guard flags.isEmpty, let index = songIndex(table.clickedRow >= 0 ? table.clickedRow : table.selectedRow) else { return }
        play(row: index)
    }

    // The actions below take song indexes, not table rows.

    func play(row: Int) {
        guard row >= 0, let songs, let shell else { return }
        shell.player.play(songAt: row, in: songs)
    }

    func toggleFavorite(row: Int) { songs?.toggleFavorite(at: row) }

    func setDownloaded(row: Int) {
        songs?.setDownloaded(at: row, true)
        (table.rowView(atRow: tableRow(row), makeIfNecessary: false) as? SongRowView).map { applyRowState($0, index: row) }
    }

    func menu(forRow row: Int) -> NSMenu? {
        guard let shell, let songs else { return nil }
        let selected = IndexSet(table.selectedRowIndexes.compactMap { songIndex($0) })
        let removing = { (rows: [Int]) in self.remove.map { remove in SongMenu.remove(count: rows.count) { remove(rows) } } }
        if selected.count > 1, selected.contains(row) {
            return makeMenu(SongMenu.several(selected.compactMap { songs.song(at: $0) }, shell: shell, remove: removing(Array(selected))))
        }
        guard let song = songs.song(at: row) else { return nil }
        return makeMenu(SongMenu.single(song, shell: shell, remove: removing([row])))
    }

    @objc private func scrolled() {
        table.refreshHover()
        checkNearEnd()
    }

    /// The first selected song, for ⌘I's Get Info.
    func tableViewSelectionDidChange(_ notification: Notification) {
        guard let shell, let songs else { return }
        shell.selectedSong = table.selectedRowIndexes.compactMap { songIndex($0) }.first.flatMap { songs.song(at: $0) }
    }

    /// The column chooser changed: every table lays its header and rows out again.
    @objc private func columnsChanged() {
        header.needsLayout = true
        table.enumerateAvailableRowViews { rowView, _ in (rowView.view(atColumn: 0) as? NSView)?.needsLayout = true }
    }

    // MARK: Sort and reordering

    func setSort(_ sort: SongSort?, descending: Bool) {
        header.sortedColumn = sort?.column
        header.descending = descending
        header.sortable = (0..<7).map { SongSort.column($0).map { songs?.canSort($0) ?? false } ?? false }
    }

    /// A click on a column header cycles that column ascending, descending, then the list's own order.
    func headerClicked(_ column: Int) {
        songs?.headerClicked(column)
        if let songs { setSort(songs.sort, descending: songs.descending) }
    }

    /// A link in a row: the artist, album or genre page.
    func open(_ route: Route) { shell?.go(route) }

    private(set) var reorder: (([Int], Int) -> Void)?
    var remove: (([Int]) -> Void)?
    /// Song indexes being dragged, from the session's start to its end.
    private(set) var draggedSongs: [Int] = []

    func setReorder(_ reorder: (([Int], Int) -> Void)?) {
        let was = self.reorder != nil
        self.reorder = reorder
        // Songs drag out of any list onto a playlist or the queue as a copy, and within a reorderable list as a move.
        // Never out of the app.
        table.setDraggingSourceOperationMask([.move, .copy], forLocal: true)
        table.setDraggingSourceOperationMask([], forLocal: false)
        guard was != (reorder != nil) else { return }
        if reorder != nil {
            table.registerForDraggedTypes([.naviaSongRow])
            // Rows slide apart to leave a gap where the drop lands, with no insertion line. `slot` marks the gap.
            table.draggingDestinationFeedbackStyle = .gap
        } else {
            table.unregisterDraggedTypes()
        }
    }

    /// Only songs can be dragged. Each carries its place in the list for reordering and its id for a playlist or the
    /// queue.
    func tableView(_ tableView: NSTableView, pasteboardWriterForRow row: Int) -> (any NSPasteboardWriting)? {
        guard let index = songIndex(row), let song = songs?.song(at: index) else { return nil }
        let item = NSPasteboardItem()
        item.setString(String(index), forType: .naviaSongRow)
        item.setString(song.id, forType: .naviaSongs)
        return item
    }

    func tableView(_ tableView: NSTableView, draggingSession session: NSDraggingSession, willBeginAt screenPoint: NSPoint, forRowIndexes rowIndexes: IndexSet) {
        draggedSongs = rowIndexes.compactMap { songIndex($0) }
        shell?.beginSongDrag(draggedSongs.compactMap { songs?.song(at: $0) })
        table.setHoverSuppressed(true)
        // The table's own drag images pile up every cell's text, so one card stands for them all.
        let rows = Array(rowIndexes)
        let local = table.convert(table.window?.convertPoint(fromScreen: screenPoint) ?? .zero, from: nil)
        let pressed = rows.contains(table.row(at: local)) ? table.row(at: local) : rows.first ?? -1
        session.animatesToStartingPositionsOnCancelOrFail = T.dragSlidesBack
        if let card = dragCard(row: pressed, count: rows.count, at: local) {
            session.draggingFormation = .none
            session.enumerateDraggingItems(options: [], for: table, classes: [NSPasteboardItem.self], searchOptions: [:]) { item, index, _ in
                item.imageComponentsProvider = nil
                item.setDraggingFrame(card.frame, contents: index == 0 ? card.image : NSImage(size: NSSize(width: 1, height: 1)))
            }
        }
        refreshDraggedRows()
    }

    func tableView(_ tableView: NSTableView, draggingSession session: NSDraggingSession, endedAt screenPoint: NSPoint, operation: NSDragOperation) {
        shell?.songDrag.end()
        draggedSongs = []
        table.setHoverSuppressed(false)
        hideSlot()
        refreshDraggedRows()
    }

    /// Where a drop lands, as the table row to drop above. Only between songs: above the headers means before the first
    /// song, past the songs means the end.
    func dropRow(proposed row: Int) -> Int? {
        guard reorder != nil, let count = songs?.count, count > 0 else { return nil }
        return min(max(row, leadRows), leadRows + count)
    }

    func tableView(_ tableView: NSTableView, validateDrop info: any NSDraggingInfo, proposedRow row: Int, proposedDropOperation dropOperation: NSTableView.DropOperation) -> NSDragOperation {
        guard (info.draggingSource as? NSTableView) === table, !draggedSongs.isEmpty, let target = dropRow(proposed: row) else { return [] }
        if target != row || dropOperation != .above { tableView.setDropRow(target, dropOperation: .above) }
        showSlot(aboveRow: target)
        return .move
    }

    // MARK: The gap's empty slot

    /// A rounded placeholder in the gap the table opens, as tall as the moved rows and inside the row's side room, so
    /// nothing draws under the sidebar or dock.
    private lazy var slot: DropSlotView = {
        let view = DropSlotView()
        view.isHidden = true
        return view
    }()
    private var slotRow: Int?

    /// AppKit slides the rows from `row` down by the dragged rows' height, leaving the gap above that row's new place.
    func gapRect(aboveRow row: Int) -> NSRect? {
        let count = max(1, draggedSongs.count)
        let height = aligned(songRowHeight) * CGFloat(count)
        guard row < table.numberOfRows else { return nil }
        let logical = table.rect(ofRow: row)
        let top = row > 0 ? table.rect(ofRow: row - 1).maxY : logical.minY
        let x = T.gutter + leadingInset
        return NSRect(x: x, y: top, width: table.bounds.width - leadingInset - trailingInset - 2 * T.gutter, height: height)
    }

    private func showSlot(aboveRow row: Int) {
        guard row != slotRow || slot.isHidden else { return }
        slotRow = row
        if slot.superview !== table { table.addSubview(slot, positioned: .below, relativeTo: nil) }
        guard let rect = gapRect(aboveRow: row) else { hideSlot(); return }
        let pill = rect.insetBy(dx: 0, dy: T.px(album ? 6.5 : 9))
        if slot.isHidden {
            slot.frame = pill
            slot.alphaValue = 0
            slot.isHidden = false
        }
        T.animate(T.fast) {
            slot.animator().frame = pill
            slot.animator().alphaValue = 1
        }
    }

    private var album: Bool { variant == .album }

    func hideSlot() {
        slotRow = nil
        slot.isHidden = true
    }

    #if DEBUG
    /// For CoreCheck's `tableGapDrag`, which has no real dragging session to start one.
    func debugBeginDrag(songs: [Int]) {
        draggedSongs = songs
        table.setHoverSuppressed(true)
        refreshDraggedRows()
    }

    func debugEndDrag() {
        draggedSongs = []
        table.setHoverSuppressed(false)
        hideSlot()
        refreshDraggedRows()
    }

    /// For CoreCheck: the slot's frame and every visible row view's frame next to its logical rect.
    func debugGap() -> [String: Any] {
        var rows: [[Double]] = []
        table.enumerateAvailableRowViews { view, row in
            let r = self.table.rect(ofRow: row)
            rows.append([Double(row), Double(view.frame.minY), Double(r.minY), Double(view.alphaValue)])
        }
        return ["slot": slot.isHidden ? [] : [Double(slot.frame.minX), Double(slot.frame.minY), Double(slot.frame.width), Double(slot.frame.height)],
                "slotRow": slotRow ?? -1, "rows": rows.sorted { $0[0] < $1[0] }.prefix(14).map { $0 }]
    }
    #endif

    func tableView(_ tableView: NSTableView, acceptDrop info: any NSDraggingInfo, row: Int, dropOperation: NSTableView.DropOperation) -> Bool {
        guard (info.draggingSource as? NSTableView) === table else { return false }
        return drop(songs: draggedSongs, aboveRow: row)
    }

    /// Moves `songs` (song indexes) above table row `row` and selects them in their new places. False when nothing moves.
    @discardableResult func drop(songs moving: [Int], aboveRow row: Int) -> Bool {
        guard let reorder, let target = dropRow(proposed: row), let count = songs?.count else { return false }
        let rows = Array(Set(moving.filter { $0 >= 0 && $0 < count })).sorted()
        let before = target - leadRows
        let insert = before - rows.filter { $0 < before }.count
        guard !rows.isEmpty, !rows.enumerated().allSatisfy({ $0.element == insert + $0.offset }) else { return false }
        reorder(rows, before)
        let landed = IndexSet(rows.indices.map { tableRow(insert + $0) })
        DispatchQueue.main.async { [weak self] in
            self?.table.selectRowIndexes(landed, byExtendingSelection: false)
        }
        return true
    }

    /// Press-and-hold on a song: begins the table's own drag with the pressed row, or the selection when the row is in it.
    func beginHoldDrag(row: Int, event: NSEvent) {
        guard songIndex(row) != nil else { return }
        if !table.selectedRowIndexes.contains(row) { table.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false) }
        let rows = table.selectedRowIndexes.filter { songIndex($0) != nil }
        let card = dragCard(row: row, count: rows.count, at: table.convert(event.locationInWindow, from: nil))
        var items: [NSDraggingItem] = []
        for r in rows {
            guard let writer = tableView(table, pasteboardWriterForRow: r) else { continue }
            let item = NSDraggingItem(pasteboardWriter: writer)
            // One card stands for every row. The others ride along unseen.
            item.setDraggingFrame(card?.frame ?? table.rect(ofRow: r), contents: r == row ? card?.image : NSImage(size: NSSize(width: 1, height: 1)))
            items.append(item)
        }
        guard !items.isEmpty else { return }
        draggedSongs = rows.compactMap { songIndex($0) }
        shell?.beginSongDrag(draggedSongs.compactMap { songs?.song(at: $0) })
        table.setHoverSuppressed(true)
        refreshDraggedRows()
        let session = table.beginDraggingSession(with: items, event: event, source: table)
        session.animatesToStartingPositionsOnCancelOrFail = T.dragSlidesBack
        session.draggingFormation = .none
    }

    /// The drag image for `count` songs led by `row`: the shared song card, below and right of `point` (default the
    /// row's leading edge). Frame in table coordinates.
    func dragCard(row: Int, count: Int, at point: NSPoint? = nil) -> (image: NSImage, frame: NSRect)? {
        guard row >= 0, let index = songIndex(row), let lead = songs?.song(at: index), let theme = shell?.theme else { return nil }
        let rect = table.rect(ofRow: row)
        return SongDragCard.dragImage(lead: lead, count: count, at: point ?? NSPoint(x: rect.minX + T.gutter, y: rect.midY), in: table, theme: theme)
    }
}

/// The empty slot in the gap a drag opens: a soft rounded fill, like an empty row.
final class DropSlotView: NSView {
    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.cornerRadius = T.rRow
        layer?.cornerCurve = .continuous
        layer?.backgroundColor = T.dragSlot.ns.cgColor
        layer?.borderColor = T.dragSlotEdge.ns.cgColor
        layer?.borderWidth = 1
    }

    required init?(coder: NSCoder) { fatalError("not used") }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

final class SongsNSTableView: NSTableView {
    weak var controller: SongsTableController?
    private(set) var hoveredRow = -1
    /// What covers the scroll view at the top and bottom, for scrolling a row into view.
    var keyboardMargins: (top: CGFloat, bottom: CGFloat) = (0, 0)
    private var area: NSTrackingArea?

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let area { removeTrackingArea(area) }
        let new = NSTrackingArea(rect: .zero, options: [.mouseMoved, .mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect], owner: self, userInfo: nil)
        addTrackingArea(new)
        area = new
    }

    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        sizeLastColumnToFit()
        controller?.tableResized()
    }

    /// Row heights are rounded to device pixels, so they change with the display's scale.
    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        controller?.backingChanged()
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil { controller?.backingChanged() }
    }

    /// The song row under a point, or -1 over the headers and the bottom room.
    func songRow(at point: NSPoint) -> Int {
        let row = row(at: point)
        return controller?.songIndex(row) != nil ? row : -1
    }

    /// Rows reach under the sidebar, the dock and the top bar, so hover only counts where a click would reach this
    /// table.
    private func hoverRow(at point: NSPoint) -> Int {
        guard let c = controller, point.x >= c.leadingInset, point.x <= bounds.width - c.trailingInset, reachable(point) else { return -1 }
        let row = row(at: point)
        return c.isSongRow(row) ? row : -1
    }

    /// Whether the window's own hit test at a point (in this view's coordinates) lands in this table.
    private func reachable(_ local: NSPoint) -> Bool {
        guard let root = window?.contentView?.superview else { return true }
        guard let hit = root.hitTest(root.convert(convert(local, to: nil), from: nil)) else { return false }
        return hit === self || hit.isDescendant(of: self)
    }

    override func mouseMoved(with event: NSEvent) {
        setHover(hoverRow(at: convert(event.locationInWindow, from: nil)))
        updateIconHover(event.locationInWindow)
    }

    /// Tells the hovered row which of its icons and links is under the pointer (window coordinates).
    private func updateIconHover(_ location: NSPoint) {
        guard hoveredRow >= 0, let cell = view(atColumn: 0, row: hoveredRow, makeIfNecessary: false) as? SongCell else { setLinkCursor(false); return }
        let local = cell.convert(location, from: nil)
        cell.iconHover(at: local)
        setLinkCursor(cell.linkHover(at: local))
    }

    private var onLink = false
    private func setLinkCursor(_ on: Bool) {
        guard on != onLink else { return }
        onLink = on
        (on ? NSCursor.pointingHand : NSCursor.arrow).set()
    }

    /// The link at a point in this table's coordinates, on a song row.
    func link(at point: NSPoint) -> Route? {
        let row = row(at: point)
        guard row >= 0, controller?.isSongRow(row) == true, let cell = view(atColumn: 0, row: row, makeIfNecessary: false) as? SongCell else { return nil }
        return cell.link(at: cell.convert(point, from: self))
    }
    override func mouseExited(with event: NSEvent) {
        setHover(-1)
        setLinkCursor(false)
    }

    func refreshHover() {
        guard let window else { return }
        let point = convert(window.mouseLocationOutsideOfEventStream, from: nil)
        setHover(visibleRect.contains(point) ? hoverRow(at: point) : -1)
        updateIconHover(window.mouseLocationOutsideOfEventStream)
    }

    /// While rows are dragged, no row shows hover.
    private var hoverSuppressed = false
    func setHoverSuppressed(_ on: Bool) {
        hoverSuppressed = on
        if on { setHover(-1) } else { refreshHover() }
    }

    override func draggingSession(_ session: NSDraggingSession, endedAt screenPoint: NSPoint, operation: NSDragOperation) {
        super.draggingSession(session, endedAt: screenPoint, operation: operation)
        setHoverSuppressed(false)
    }

    /// AppKit opens and closes the gap itself, through `T.appKitMotion` so it's instant while motion is reduced.
    override func draggingEntered(_ sender: any NSDraggingInfo) -> NSDragOperation {
        T.appKitMotion { super.draggingEntered(sender) }
    }

    override func draggingUpdated(_ sender: any NSDraggingInfo) -> NSDragOperation {
        T.appKitMotion { super.draggingUpdated(sender) }
    }

    /// A drag that leaves the table closes its gap, so the empty slot goes too.
    override func draggingExited(_ sender: (any NSDraggingInfo)?) {
        T.appKitMotion { super.draggingExited(sender) }
        controller?.hideSlot()
    }

    private func setHover(_ row: Int) {
        let row = hoverSuppressed ? -1 : row
        guard row != hoveredRow else { return }
        apply(hoveredRow, on: false)
        hoveredRow = row
        apply(row, on: true)
    }

    private func apply(_ row: Int, on: Bool) {
        guard row >= 0, row < numberOfRows else { return }
        (rowView(atRow: row, makeIfNecessary: false) as? SongRowView)?.isHovered = on
        (view(atColumn: 0, row: row, makeIfNecessary: false) as? SongCell)?.setHovered(on)
    }

    #if DEBUG
    /// Mouse downs that reached the table, for CoreCheck's `rowProbe`.
    static var debugMouseDowns = 0
    #endif

    /// Control-click adds to the selection. Right-click still opens the menu.
    override func mouseDown(with event: NSEvent) {
        #if DEBUG
        Self.debugMouseDowns += 1
        #endif
        window?.makeFirstResponder(self)
        // A double-click on rows scrolled under the top bar belongs to the title bar.
        if TitlebarDoubleClick.handleInTopBar(event) { return }
        // A plain click on a link's text follows it and leaves the selection alone.
        if event.modifierFlags.intersection([.command, .shift, .control]).isEmpty, event.clickCount == 1,
           let route = link(at: convert(event.locationInWindow, from: nil)) {
            controller?.open(route)
            return
        }
        // A press held still for 180 ms lifts the row, or the selection it's in, for reordering or dragging to a playlist.
        // A release or move sooner goes to the table as usual.
        let plain = event.modifierFlags.intersection([.command, .control, .shift, .option]).isEmpty
        if plain, event.clickCount == 1, let controller, (controller.songs?.count ?? 0) > 0 {
            let row = songRow(at: convert(event.locationInWindow, from: nil))
            if row >= 0, let window,
               window.nextEvent(matching: [.leftMouseUp, .leftMouseDragged], until: Date(timeIntervalSinceNow: RowHold.holdSeconds), inMode: .eventTracking, dequeue: false) == nil,
               NSEvent.pressedMouseButtons & 1 == 1 {
                controller.beginHoldDrag(row: row, event: event)
                return
            }
        }
        if event.modifierFlags.contains(.control), !event.modifierFlags.contains(.shift) {
            let row = songRow(at: convert(event.locationInWindow, from: nil))
            guard row >= 0 else { return }
            if selectedRowIndexes.contains(row) { deselectRow(row) } else { selectRowIndexes(IndexSet(integer: row), byExtendingSelection: true) }
            return
        }
        super.mouseDown(with: event)
    }

    /// A row counts as visible only clear of the top bar and bottom margin, so arrow keys never select a hidden row.
    override func scrollRowToVisible(_ row: Int) {
        #if DEBUG
        TableCheck.scrollRowCalls += 1
        #endif
        guard row >= 0, row < numberOfRows, let clip = enclosingScrollView?.contentView else { return super.scrollRowToVisible(row) }
        let r = clip.convert(rect(ofRow: row), from: self)
        let top = clip.bounds.minY + keyboardMargins.top, bottom = clip.bounds.maxY - keyboardMargins.bottom
        var y = clip.bounds.minY
        if r.minY < top { y -= top - r.minY } else if r.maxY > bottom { y += r.maxY - bottom } else { return }
        clip.scroll(to: clip.constrainBoundsRect(NSRect(origin: NSPoint(x: clip.bounds.minX, y: y), size: clip.bounds.size)).origin)
        enclosingScrollView?.reflectScrolledClipView(clip)
    }

    override func menu(for event: NSEvent) -> NSMenu? {
        if event.type == .leftMouseDown { return nil } // A control-click is handled in mouseDown.
        if let index = controller?.trailingIndex(row(at: convert(event.locationInWindow, from: nil))) { return controller?.trailingMenu(index) }
        let row = songRow(at: convert(event.locationInWindow, from: nil))
        guard row >= 0, let index = controller?.songIndex(row) else { return nil }
        if !selectedRowIndexes.contains(row) { selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false) }
        return controller?.menu(forRow: index)
    }

    override func keyDown(with event: NSEvent) {
        if event.keyCode == 36 || event.keyCode == 76 { // Return and Enter
            if let index = controller?.songIndex(selectedRow) { controller?.play(row: index) }
        } else {
            super.keyDown(with: event)
        }
    }
}

/// The page header's room, the column header row and the bottom room. Never selected. The page header's row lets
/// clicks through to the hosted header.
final class SpacerRowView: NSTableRowView {
    override func drawBackground(in dirtyRect: NSRect) {}
    override func drawSelection(in dirtyRect: NSRect) {}
    override func drawSeparator(in dirtyRect: NSRect) {}
    /// Only the cell takes a point, not the row or AppKit's separator view under it.
    override func hitTest(_ point: NSPoint) -> NSView? {
        let local = convert(point, from: superview)
        guard numberOfColumns > 0, let cell = view(atColumn: 0) as? NSView, !cell.isHidden, cell.frame.contains(local) else { return nil }
        return cell.hitTest(local)
    }
}

/// Draws the inset rounded selection and hover pills, and the row separator.
final class SongRowView: NSTableRowView {
    var colors = TableColors(accent: Theme.defaultAccent, accentSoft: Theme.defaultAccent, onAccent: RGBA(0, 0, 0))
    var compact = false { didSet { if compact != oldValue { needsDisplay = true } } }
    var album = false { didSet { if album != oldValue { needsDisplay = true } } }
    var isDownloaded = false { didSet { if isDownloaded != oldValue { needsDisplay = true } } }
    var isHovered = false { didSet { if isHovered != oldValue { needsDisplay = true } } }
    var leadingInset: CGFloat = 0
    var trailingInset: CGFloat = 0
    var hidesSeparator = false { didSet { if hidesSeparator != oldValue { needsDisplay = true } } }

    private var innerRect: NSRect {
        NSRect(x: T.gutter + leadingInset, y: bounds.minY, width: bounds.width - leadingInset - trailingInset - 2 * T.gutter, height: bounds.height)
    }

    private var pillRect: NSRect {
        let vertical = album ? T.px(6.5) : compact ? T.px(3) : T.px(13)
        return innerRect.insetBy(dx: 0, dy: vertical)
    }

    override func drawBackground(in dirtyRect: NSRect) {
        var fill: RGBA?
        if isDownloaded { fill = isHovered ? T.hover : T.row } else if isHovered { fill = T.row }
        if let fill, !isSelected {
            fill.ns.setFill()
            NSBezierPath(roundedRect: pillRect, xRadius: T.rRow, yRadius: T.rRow).fill()
        }
        guard !hidesSeparator else { return }
        T.line.ns.setFill()
        NSRect(x: innerRect.minX, y: bounds.maxY - 1, width: innerRect.width, height: 1).fill()
    }

    override func drawSelection(in dirtyRect: NSRect) {
        // The accent at 30%, with the row's own text colors on top.
        colors.rowSelected.ns.setFill()
        NSBezierPath(roundedRect: pillRect, xRadius: T.rRow, yRadius: T.rRow).fill()
    }

    override var isSelected: Bool { didSet { needsDisplay = true } }
    override var isEmphasized: Bool { didSet { needsDisplay = true } }
    override var interiorBackgroundStyle: NSView.BackgroundStyle { .normal }
}

/// The column header row's cell, so it scrolls away with the songs.
final class SongHeaderView: NSView {
    var compact = false
    var variant: SongListVariant = .full
    var leadingInset: CGFloat = 0
    var trailingInset: CGFloat = 0
    /// The sorted column (0 name, 1 time, 2 artist, 3 album, 4 genre, 5 date added, 6 plays), shown brighter. Nil for
    /// none.
    var sortedColumn: Int? { didSet { if sortedColumn != oldValue { applyEmphasis() } } }
    var descending = false { didSet { if descending != oldValue { applyEmphasis() } } }
    /// Which headers sort when clicked.
    var sortable: [Bool] = []
    weak var controller: SongsTableController?
    private let titles = [L("Song Name"), L("Time"), L("Artist name"), L("Album name"), L("Genre"), L("Date Added"), L("Plays")]
    private let labels: [NSTextField] = (0..<7).map { i in
        let field = SongCell.makeLabel()
        if i == 6 { field.alignment = .right }
        return field
    }

    override var isFlipped: Bool { true }

    init() {
        super.init(frame: .zero)
        labels.forEach(addSubview)
        applyEmphasis()
    }

    /// Right-click: the columns that can be shown or hidden, saved per account.
    override func menu(for event: NSEvent) -> NSMenu? {
        guard variant == .full else { return nil }
        let shown = SongColumn.shown
        return makeMenu(SongColumn.allCases.map { column in
            MenuEntry(title: column.label, checked: shown.contains(column)) { SongColumn.toggle(column) }
        })
    }

    override func rightMouseDown(with event: NSEvent) {
        guard let menu = menu(for: event) else { return }
        NSMenu.popUpContextMenu(menu, with: event, for: self)
    }

    /// Headers are dim. The sorted column is bright and medium weight, with an arrow after it.
    private func applyEmphasis() {
        for (i, label) in labels.enumerated() {
            let on = i == sortedColumn
            label.textColor = (on ? T.tEm : T.t2).ns
            label.font = .systemFont(ofSize: T.textMD, weight: on ? .medium : .regular)
            label.stringValue = on ? "\(titles[i]) \(descending ? "▼" : "▲")" : titles[i]
        }
    }

    /// The header takes all its clicks, so no row is selected and no drag starts.
    override func hitTest(_ point: NSPoint) -> NSView? {
        frame.contains(point) ? self : nil
    }

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func mouseDown(with event: NSEvent) {
        let p = convert(event.locationInWindow, from: nil)
        guard let column = column(at: p) else { return }
        controller?.headerClicked(column)
    }

    /// A header label's frame, for checks.
    func labelFrame(_ column: Int) -> NSRect { labels.indices.contains(column) ? labels[column].frame : .zero }

    /// The header column under a point: each label's column runs to the next label.
    func column(at p: NSPoint) -> Int? {
        let shown = labels.enumerated().filter { !$0.element.isHidden }
        for (k, item) in shown.enumerated() {
            let start = item.element.frame.minX
            let end = k + 1 < shown.count ? shown[k + 1].element.frame.minX : item.element.frame.maxX + T.px(14)
            if p.x >= start && p.x < end { return sortable.indices.contains(item.offset) && sortable[item.offset] ? item.offset : nil }
        }
        return nil
    }

    override func resetCursorRects() {
        for (i, label) in labels.enumerated() where !label.isHidden && sortable.indices.contains(i) && sortable[i] {
            addCursorRect(label.frame, cursor: .pointingHand)
        }
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    override func layout() {
        super.layout()
        needsDisplay = true // The rule under the labels follows the width and insets.
        let width = bounds.width - 2 * T.gutter - leadingInset - trailingInset
        let columns = SongColumns(width: width, compact: compact, variant: variant)
        // Song Name starts where the artwork does.
        let left = T.gutter + leadingInset
        let artLeft = left + T.px(5) + (compact ? T.px(12) : T.px(21)) - T.thumb / 2
        var x = artLeft
        let height = ceil(labels[0].intrinsicContentSize.height)
        let y = (bounds.height - height) / 2
        // Labels take their column's full width, so a short one like Time still fits its sort arrow.
        func place(_ field: NSTextField, _ w: CGFloat) {
            field.isHidden = w <= 0
            field.frame = NSRect(x: x, y: y, width: max(0, w - T.px(4)), height: height)
            x += w
        }
        place(labels[0], left + columns.lead + columns.title - artLeft)
        place(labels[1], columns.time)
        place(labels[2], columns.artist)
        place(labels[3], columns.album)
        place(labels[4], columns.genre)
        place(labels[5], columns.added)
        place(labels[6], columns.plays)
        // Plays lines up with its numbers, which end 20 points in from the column's edge.
        if columns.plays > 0 { labels[6].frame.size.width = max(0, columns.plays - T.px(20)) }
    }

    override func draw(_ dirtyRect: NSRect) {
        T.line.ns.setFill()
        NSRect(x: T.gutter + leadingInset, y: bounds.maxY - 1, width: bounds.width - 2 * T.gutter - leadingInset - trailingInset, height: 1).fill()
    }
}

/// One song row. Built by hand with frames so reuse and layout stay cheap at any library size.
final class SongCell: NSTableCellView {
    private let thumb = NSImageView()
    private let playOverlay = HoverPlayButton()
    private let number = SongCell.makeLabel()
    private let name = SongCell.makeLabel()
    private let time = SongCell.makeLabel()
    private let artist = SongCell.makeLabel()
    private let album = SongCell.makeLabel()
    private let genre = SongCell.makeLabel()
    private let added = SongCell.makeLabel()
    private let plays = SongCell.makeLabel()
    private let explicit = ExplicitBadgeView()
    private var isExplicit = false
    private let download = SongCell.makeAction("arrow.down", label: L("Download"))
    private let star = SongCell.makeAction("heart", label: L("Favorite"))
    private let more = SongCell.makeAction("ellipsis", label: L("More"))

    private var row = -1
    private var isNow = false
    /// A song after the list: Add in place of Download and Favorite.
    private var trailing = false
    private var addHost: NSHostingView<AnyView>?
    /// Artist, album and genre links: the field, the linked part of its text, and where it goes.
    private var links: [(field: NSTextField, range: NSRange, route: Route)] = []
    private var hoveredLink: Int?
    private var compact = false
    private var variant: SongListVariant = .full
    private var leadingInset: CGFloat = 0
    private var trailingInset: CGFloat = 0
    private var isFavorite = false
    private var isDownloaded = false
    private var colors = TableColors(accent: Theme.defaultAccent, accentSoft: Theme.defaultAccent, onAccent: RGBA(0, 0, 0))
    private var artworkURL: URL?
    private weak var controller: SongsTableController?

    override var isFlipped: Bool { true }

    static func makeLabel() -> NSTextField {
        let field = NSTextField(labelWithString: "")
        field.lineBreakMode = .byTruncatingTail
        field.maximumNumberOfLines = 1
        field.usesSingleLineMode = true
        field.font = .systemFont(ofSize: T.textMD)
        field.textColor = T.t2.ns
        return field
    }

    private static func makeAction(_ symbol: String, label: String) -> RowActionButton {
        let button = RowActionButton(frame: .zero)
        button.isBordered = false
        button.bezelStyle = .accessoryBarAction
        button.imagePosition = .imageOnly
        button.imageScaling = .scaleNone
        button.image = symbolImage(symbol, label: label)
        button.toolTip = label
        return button
    }

    fileprivate static func symbolImage(_ symbol: String, label: String, weight: NSFont.Weight = .regular, size: CGFloat = T.px(17)) -> NSImage? {
        NSImage(systemSymbolName: symbol, accessibilityDescription: label)?
            .withSymbolConfiguration(.init(pointSize: size, weight: weight))
    }

    init() {
        super.init(frame: .zero)
        wantsLayer = true
        thumb.wantsLayer = true
        thumb.layer?.cornerRadius = T.rSM
        thumb.layer?.cornerCurve = .continuous
        thumb.layer?.masksToBounds = true
        thumb.layer?.backgroundColor = T.panel2.ns.cgColor
        thumb.imageScaling = .scaleProportionallyUpOrDown
        name.font = .systemFont(ofSize: T.textMD, weight: .medium)
        time.font = .monospacedDigitSystemFont(ofSize: T.textMD, weight: .regular)
        plays.font = .monospacedDigitSystemFont(ofSize: T.textMD, weight: .regular)
        plays.alignment = .right
        number.font = .monospacedDigitSystemFont(ofSize: T.textMD, weight: .regular)
        number.alignment = .center
        for button in actions {
            button.onPress = { [weak self] in self?.refreshIconHover() }
        }
        [thumb, number, playOverlay, name, explicit, time, artist, album, genre, added, plays, download, star, more].forEach(addSubview)
        playOverlay.isHidden = true
        playOverlay.target = self
        playOverlay.action = #selector(playTapped)
        download.target = self
        download.action = #selector(downloadTapped)
        star.target = self
        star.action = #selector(starTapped)
        more.target = self
        more.action = #selector(moreTapped)
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    func configure(song: SongRow, row: Int, isNow: Bool, compact: Bool, variant: SongListVariant, colors: TableColors, leadingInset: CGFloat, trailingInset: CGFloat, controller: SongsTableController, trailing: Bool = false) {
        self.row = row
        self.trailing = trailing
        if trailing {
            let add = AnyView(ActionButton(title: L("Add"), small: true) { [weak controller] in controller?.addTrailing(row) }
                .accessibilityLabel(L("Add {name} to This Playlist", ["name": song.title])))
            if let addHost { addHost.rootView = add } else {
                let host = NSHostingView(rootView: add)
                host.sizingOptions = [.intrinsicContentSize]
                addSubview(host)
                addHost = host
            }
        }
        addHost?.isHidden = !trailing
        star.isHidden = trailing
        self.isNow = isNow
        self.compact = compact
        self.variant = variant
        self.leadingInset = leadingInset
        self.trailingInset = trailingInset
        self.colors = colors
        self.controller = controller
        isFavorite = song.isFavorite
        isDownloaded = song.isDownloaded
        name.stringValue = song.title
        time.stringValue = clock(Double(song.duration))
        artist.stringValue = song.artist
        album.stringValue = song.album
        genre.stringValue = song.genre
        added.stringValue = fmtDate(song.added)
        plays.stringValue = song.plays > 0 ? fmtInt(song.plays) : ""
        isExplicit = song.isExplicit
        links = SongLinks.artists(song).map { (artist, NSRange($0.range, in: song.artist), Route.artist($0.id)) }
        if !song.albumID.isEmpty, !song.album.isEmpty { links.append((album, NSRange(location: 0, length: (song.album as NSString).length), .album(song.albumID))) }
        if !song.genre.isEmpty { links.append((genre, NSRange(location: 0, length: (song.genre as NSString).length), .songs(genre: song.genre))) }
        hoveredLink = nil
        download.isHidden = song.isDownloaded || trailing
        let dim = controller.offline && controller.shell?.library.playsOffline(song: song) != true
        [thumb, number, name, explicit, time, artist, album, genre, added, plays].forEach { $0.alphaValue = dim ? 0.4 : 1 }
        download.image = Self.symbolImage(controller.shell?.library.isDownloading(song: song) == true ? "arrow.down.circle.dotted" : "arrow.down", label: L("Download"))
        star.image = Self.symbolImage(song.isFavorite ? "heart.fill" : "heart", label: L("Favorite"))
        number.stringValue = song.trackNumber.map(String.init) ?? "\(row + 1)"
        number.isHidden = variant != .album
        thumb.isHidden = compact || variant == .album
        loadArtwork(song.artworkURL)
        applyColors()
        needsLayout = true
    }

    /// A row that hasn't loaded yet, so a reused cell doesn't show another song.
    func clear(row: Int) {
        self.row = row
        [name, artist, album, genre, added, plays, time, number].forEach { $0.stringValue = "" }
        isExplicit = false
        links = []
        hoveredLink = nil
        star.image = nil
        [thumb, number, name, explicit, time, artist, album, genre, added, plays].forEach { $0.alphaValue = 1 }
        artworkURL = nil
        thumb.image = nil
        thumb.isHidden = false
        needsLayout = true
    }

    private func loadArtwork(_ url: URL?) {
        guard !compact, variant != .album else { artworkURL = nil; thumb.image = nil; return }
        guard url != artworkURL || thumb.image == nil else { return }
        artworkURL = url
        thumb.image = nil
        guard let url else { return }
        let pixels = Int(T.thumb * 2)
        if let hit = ArtworkLoader.shared.cached(url, pixels: pixels) { thumb.image = hit; return }
        Task { [weak self] in
            var delay = 2.0
            for _ in 0..<8 {
                let image = await ArtworkLoader.shared.load(url, pixels: pixels)
                guard let self, self.artworkURL == url else { return }
                if let image {
                    self.thumb.image = image
                    return
                }
                try? await Task.sleep(for: .seconds(delay))
                delay = min(delay * 2, 60)
            }
        }
    }

    func setHovered(_ on: Bool) {
        playOverlay.isHidden = !on
        if variant == .album { number.isHidden = on }
        if !on { iconHover(at: nil) }
        if !on { linkHover(at: nil) }
    }

    /// The link under a point, only if the point is on the linked text itself.
    func link(at point: NSPoint) -> Route? { linkIndex(at: point).map { links[$0].route } }

    private func linkIndex(at point: NSPoint) -> Int? {
        for (i, link) in links.enumerated() where !link.field.isHidden && !link.field.stringValue.isEmpty {
            let frame = link.field.frame
            guard point.y >= frame.minY, point.y <= frame.maxY else { continue }
            let text = link.field.stringValue as NSString
            let attrs: [NSAttributedString.Key: Any] = [.font: link.field.font ?? .systemFont(ofSize: T.textMD)]
            // A label draws its text 2 pt in from its frame.
            let x0 = frame.minX + 2 + text.substring(to: link.range.location).size(withAttributes: attrs).width
            let x1 = min(frame.maxX - 2, x0 + text.substring(with: link.range).size(withAttributes: attrs).width)
            if point.x >= x0, point.x <= x1 { return i }
        }
        return nil
    }

    #if DEBUG
    /// Each link's text span in window points from the top left, with where it goes, for CoreCheck.
    func debugLinkSpans() -> [[String: Any]] {
        guard let window else { return [] }
        let height = window.contentView?.bounds.height ?? 0
        return links.indices.compactMap { i in
            let frame = links[i].field.frame
            // Find the span by probing across the field, the same test a click uses.
            let xs = stride(from: frame.minX, through: frame.maxX, by: 1).filter { linkIndex(at: NSPoint(x: $0, y: frame.midY)) == i }
            guard let lo = xs.first, let hi = xs.last else { return nil }
            let a = convert(NSPoint(x: lo, y: frame.midY), to: nil), b = convert(NSPoint(x: hi, y: frame.midY), to: nil)
            return ["x0": Double(a.x), "x1": Double(b.x), "y": Double(height - a.y), "route": "\(links[i].route)"]
        }
    }
    #endif

    /// Underlines the linked text under the pointer. Returns whether the pointer is on a link.
    @discardableResult func linkHover(at point: NSPoint?) -> Bool {
        let index = point.flatMap(linkIndex(at:))
        if index != hoveredLink {
            hoveredLink = index
            applyLinkStyle()
        }
        return index != nil
    }

    private func applyLinkStyle() {
        for field in [artist, album, genre] {
            let style = NSMutableParagraphStyle()
            style.lineBreakMode = .byTruncatingTail
            let text = NSMutableAttributedString(string: field.stringValue, attributes: [.font: field.font ?? .systemFont(ofSize: T.textMD), .foregroundColor: T.t2.ns, .paragraphStyle: style])
            if let hoveredLink, links[hoveredLink].field === field {
                text.addAttribute(.underlineStyle, value: NSUnderlineStyle.single.rawValue, range: links[hoveredLink].range)
                text.addAttribute(.foregroundColor, value: T.t1.ns, range: links[hoveredLink].range)
            }
            field.attributedStringValue = text
        }
    }

    private var actions: [RowActionButton] { [download, star, more] }
    private var pointer: NSPoint?

    /// A circle behind the icon under the pointer, as `HoverFill` does. The table's tracking drives it with the point in
    /// this cell's coordinates, or nil when the pointer leaves.
    func iconHover(at point: NSPoint?) {
        pointer = point
        refreshIconHover()
    }

    private func refreshIconHover() {
        for button in actions {
            let over = pointer.map { button.frame.contains($0) } ?? false
            button.setHover(!button.isHidden && button.isEnabled && (over || HoverDebug.forced("RowIcons")))
        }
    }

    private func applyColors() {
        name.textColor = (isNow ? colors.accentSoft : T.t1).ns
        [time, artist, album, genre, added, plays, number].forEach { $0.textColor = T.t2.ns }
        let icon = T.t1.ns
        [download, more].forEach { $0.restTint = icon.withAlphaComponent(0.88) }
        star.restTint = isFavorite ? icon : icon.withAlphaComponent(0.88)
    }

    override func layout() {
        super.layout()
        let left = T.gutter + leadingInset
        let width = bounds.width - 2 * T.gutter - leadingInset - trailingInset
        let columns = SongColumns(width: width, compact: compact, variant: variant)
        let h = bounds.height
        let lineHeight = ceil(name.intrinsicContentSize.height)
        let textY = (h - lineHeight) / 2

        let leadCenter = left + T.px(5) + (compact ? T.px(12) : T.px(21))
        thumb.frame = NSRect(x: leadCenter - T.thumb / 2, y: (h - T.thumb) / 2, width: T.thumb, height: T.thumb)
        playOverlay.frame = NSRect(x: leadCenter - T.thumb / 2, y: (h - T.thumb) / 2, width: T.thumb, height: T.thumb)
        number.frame = NSRect(x: leadCenter - T.thumb / 2, y: textY, width: T.thumb, height: lineHeight)

        var x = left + columns.lead
        func place(_ field: NSTextField, _ w: CGFloat, pad: CGFloat) {
            field.isHidden = w <= 0
            field.frame = NSRect(x: x, y: textY, width: max(0, w - pad), height: lineHeight)
            x += w
        }
        place(name, columns.title, pad: T.px(12))
        let twoLine = variant == .album
        if twoLine {
            // An album's track: the title over its artists, every one a link, with the explicit mark before them.
            let line = T.textMD * 1.32
            let top = (h - 2 * line) / 2
            name.frame.origin.y = top + (line - lineHeight) / 2
            let side = ceil(T.textMD * 0.85)
            let mark = isExplicit ? side + T.textMD * 0.3 : 0
            explicit.isHidden = !isExplicit
            explicit.frame = NSRect(x: name.frame.minX + 2, y: top + line + (line - side) / 2, width: side, height: side)
            artist.isHidden = false
            artist.frame = NSRect(x: name.frame.minX + mark, y: top + line + (line - lineHeight) / 2, width: max(0, name.frame.width - mark), height: lineHeight)
        }
        // The explicit mark follows the title, and a long title truncates before it.
        if !twoLine { explicit.isHidden = !isExplicit || name.isHidden }
        if isExplicit, !name.isHidden, !twoLine {
            let side = ceil(T.textMD * 0.85)
            let gap = T.textMD * 0.27
            let room = max(0, name.frame.width - side - gap)
            let textWidth = min(room, ceil(name.attributedStringValue.size().width) + 4)
            name.frame.size.width = textWidth
            explicit.frame = NSRect(x: name.frame.minX + textWidth + gap - 2, y: (h - side) / 2, width: side, height: side)
        }
        place(time, columns.time, pad: T.px(14))
        if twoLine { x += columns.artist } else { place(artist, columns.artist, pad: T.px(14)) }
        place(album, columns.album, pad: T.px(14))
        place(genre, columns.genre, pad: T.px(14))
        place(added, columns.added, pad: T.px(14))
        place(plays, columns.plays, pad: T.px(20))

        let buttonH = T.iconH
        let y = (h - buttonH) / 2
        var right = bounds.width - T.gutter - trailingInset - T.px(12)
        for button in [more, star, download] {
            right -= T.actionW
            button.frame = NSRect(x: right, y: y, width: T.actionW, height: buttonH)
        }
        if trailing, let addHost {
            addHost.invalidateIntrinsicContentSize()
            let size = addHost.intrinsicContentSize
            addHost.frame = NSRect(x: more.frame.minX - size.width, y: (h - size.height) / 2, width: size.width, height: size.height)
        }
        refreshIconHover()
    }

    @objc private func playTapped() { if trailing { controller?.playTrailing(row) } else { controller?.play(row: row) } }
    @objc private func starTapped() { controller?.toggleFavorite(row: row) }
    @objc private func downloadTapped() { controller?.setDownloaded(row: row) }
    @objc private func moreTapped() {
        guard let menu = trailing ? controller?.trailingMenu(row) : controller?.menu(forRow: row) else { return }
        menu.popUp(positioning: nil, at: NSPoint(x: 0, y: more.bounds.maxY + 4), in: more)
    }
}

/// A row icon whose ink brightens on hover and dims while pressed.
final class RowActionButton: NSButton {
    var onPress: (() -> Void)?
    /// The icon's ink at rest.
    var restTint: NSColor = T.t1.ns { didSet { update() } }
    private(set) var hovered = false
    private(set) var pressed = false

    func setHover(_ on: Bool) {
        guard on != hovered else { return }
        hovered = on
        update()
    }

    private func update() {
        contentTintColor = hovered ? T.hoverInk.ns : restTint
        alphaValue = pressed && hovered ? T.pressDim : 1
    }

    override func mouseDown(with event: NSEvent) {
        pressed = true
        update()
        super.mouseDown(with: event) // Tracks until the button is released.
        pressed = false
        update()
        onPress?()
    }

}

/// The play glyph shown over the artwork of the hovered row.
final class HoverPlayButton: NSButton {
    init() {
        super.init(frame: .zero)
        isBordered = false
        bezelStyle = .accessoryBarAction
        imagePosition = .imageOnly
        imageScaling = .scaleNone
        image = SongCell.symbolImage("play.fill", label: L("Play"), size: T.px(15))
        contentTintColor = T.tOnArt.ns
        toolTip = L("Play")
        wantsLayer = true
        shadow = {
            let s = NSShadow()
            s.shadowColor = T.hoverPlayShadow.color.ns
            s.shadowBlurRadius = T.hoverPlayShadow.blur
            s.shadowOffset = .init(width: T.hoverPlayShadow.x, height: -T.hoverPlayShadow.y)
            return s
        }()
    }

    required init?(coder: NSCoder) { fatalError("not used") }
}

/// The explicit "E" for AppKit rows: a dim rounded square with a bold E, sized from the title's font.
final class ExplicitBadgeView: NSView {
    var fontSize: CGFloat = T.textMD { didSet { needsDisplay = true } }

    override init(frame: NSRect) {
        super.init(frame: frame)
        toolTip = L("Explicit")
        setAccessibilityElement(true)
        setAccessibilityRole(.image)
        setAccessibilityLabel(L("Explicit"))
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    override func draw(_ dirtyRect: NSRect) {
        T.explicitFill.ns.setFill()
        NSBezierPath(roundedRect: bounds, xRadius: 3, yRadius: 3).fill()
        let text = NSAttributedString(string: "E", attributes: [.font: NSFont.systemFont(ofSize: fontSize * 0.68, weight: .bold),
                                                                 .foregroundColor: T.explicitInk.ns])
        let size = text.size()
        text.draw(at: NSPoint(x: bounds.midX - size.width / 2, y: bounds.midY - size.height / 2))
    }
}

/// A disc's header: a disc icon and the disc's name, or "Disc 2", with the row rule under it.
final class DiscHeaderView: NSView {
    private let icon = NSImageView()
    private let label = SongCell.makeLabel()
    private var leadingInset: CGFloat = 0
    private var trailingInset: CGFloat = 0

    override var isFlipped: Bool { true }

    init() {
        super.init(frame: .zero)
        icon.image = NSImage(systemSymbolName: "opticaldisc", accessibilityDescription: nil)?
            .withSymbolConfiguration(.init(pointSize: T.px(15), weight: .regular))
        icon.contentTintColor = T.t2.ns
        label.font = .systemFont(ofSize: T.textMD, weight: .medium)
        label.textColor = T.t2.ns
        addSubview(icon)
        addSubview(label)
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    func configure(title: String, leadingInset: CGFloat, trailingInset: CGFloat) {
        label.stringValue = title
        setAccessibilityLabel(title)
        self.leadingInset = leadingInset
        self.trailingInset = trailingInset
        needsLayout = true
        needsDisplay = true
    }

    /// Takes no clicks, so a click on it doesn't select a row.
    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func layout() {
        super.layout()
        let left = T.gutter + leadingInset + T.px(17)
        let top = T.px(14)
        let side = T.px(18)
        let mid = top + (bounds.height - top) / 2
        icon.frame = NSRect(x: left, y: mid - side / 2, width: side, height: side)
        let h = ceil(label.intrinsicContentSize.height)
        let x = icon.frame.maxX + T.px(10)
        label.frame = NSRect(x: x, y: mid - h / 2, width: max(0, bounds.width - T.gutter - trailingInset - x), height: h)
    }

    override func draw(_ dirtyRect: NSRect) {
        T.line.ns.setFill()
        let x = T.gutter + leadingInset
        NSRect(x: x, y: bounds.maxY - 1, width: bounds.width - 2 * T.gutter - leadingInset - trailingInset, height: 1).fill()
    }
}
