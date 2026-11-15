import SwiftUI

struct SidebarView: View {
    let shell: ShellModel
    /// The width in points, from `ShellModel.sidebarPoints`.
    let width: CGFloat
    let toggle: () -> Void
    @State private var libraryOpen = true
    @State private var playlistsOpen = true
    @State private var sharedOpen = true
    @State private var scrollPosition = ScrollPosition(y: 0)
    @State private var scrollTop: CGFloat = 0
    @State private var scrollHeight: CGFloat = 0
    @State private var contentHeight: CGFloat = 0
    @Environment(\.isSnapshot) private var isSnapshot

    private let top: [(Route, String, String)] = [
        (.browse(), L("Search"), "magnifyingglass"), (.home, L("Home"), "house"),
        (.discover(.new), L("New"), "square.grid.2x2"), (.discover(.radio), L("Radio"), "dot.radiowaves.left.and.right"),
    ]
    private let library: [(Route, String, String)] = [
        // Liked Songs first. Recently Added is a sort on Albums and Songs.
        (.liked, L("Liked Songs"), "heart"), (.songs(genre: nil), L("Songs"), "music.note"),
        (.artists, L("Artists"), "music.microphone"), (.albums(.all), L("Albums"), "square.stack"), (.browse(genres: true), L("Genres"), "guitars"),
    ]

    var body: some View {
        VStack(spacing: 0) {
            if isSnapshot {
                header
                rows.frame(minHeight: 0, maxHeight: .infinity, alignment: .top).clipped()
            } else {
                // Rows blur and fade as they reach the window buttons. The header acts as the bar, so rows start below it.
                ScrollView(.vertical) { rows.coordinateSpace(.named(SidebarPlaylists.space)) }
                    .scrollPosition($scrollPosition)
                    .onScrollGeometryChange(for: CGRect.self, of: { CGRect(x: 0, y: $0.contentOffset.y, width: $0.containerSize.height, height: $0.contentSize.height) }) { _, g in
                        scrollTop = g.minY
                        scrollHeight = g.width
                        contentHeight = g.height
                        SidebarPlaylists.scrollTo = { y in scrollPosition.scrollTo(y: y) }
                    }
                    // Songs carried near the top or bottom edge scroll it. Scrolling goes through the scroll position so it stays put.
                    .onScrollGeometryChange(for: [CGFloat].self, of: { g in
                        [g.contentOffset.y + g.contentInsets.top, max(0, g.contentSize.height - g.containerSize.height + g.contentInsets.top + g.contentInsets.bottom)]
                    }) { _, at in
                        shell.songDrag.sidebarScrollBy = { delta in scrollPosition.scrollTo(y: min(at[1], max(0, at[0] + delta))) }
                    }
                    .scrollIndicators(.never)
                    .scrollEdgeEffectStyle(.soft, for: .top)
                    .safeAreaBar(edge: .top, spacing: 0) { header }
            }

            // Settings lives in the account button, so the playlists end the sidebar.
            if let progress = shell.library.syncProgress, !shell.library.syncMeterOnPage {
                SyncCard(progress: progress, fill: shell.theme.syncFill).padding([.horizontal, .top], T.px(12)).padding(.top, -T.px(6)).padding(.bottom, T.px(12))
            }
            if shell.library.isOffline { OfflineRow(shell: shell).padding([.horizontal, .top], T.px(12)).padding(.top, -T.px(6)).padding(.bottom, T.px(12)) }
        }
        .frame(width: width)
        .frame(maxHeight: .infinity)
        // The scroll view's soft top edge reaches the window's top, so clip it to the panel. Unclipped, it lightened the
        // margin above the panel and showed as a line along the corner.
        .clipShape(RoundedRectangle(cornerRadius: T.rLG, style: .continuous))
        .panelGlass(radius: T.rLG)
    }

    /// The sidebar toggle's row, which also clears the window buttons.
    private var header: some View {
        HStack {
            Spacer()
            IconButton(symbol: "sidebar.left", label: L("Hide sidebar"), action: toggle)
        }
        .padding(.trailing, T.px(12))
        .frame(height: T.px(62))
    }

    private var rows: some View {
        VStack(spacing: 0) {
            ForEach(top, id: \.1) { item(route: $0.0, label: $0.1, symbol: $0.2) }
            heading(L("Library"), open: $libraryOpen)
            if libraryOpen { ForEach(library, id: \.1) { item(route: $0.0, label: $0.1, symbol: $0.2) } }
            // Listening Now shows others who share while this listener shares too, then Spotify friends who are listening.
            if let listen = shell.player.listen {
                let people = listen.active ? listen.listing : []
                let friends = listen.listeningFriends
                if !people.isEmpty || !friends.isEmpty {
                    headingLine(L("Listening Now")) { EmptyView() }
                    ForEach(people, id: \.user) { PersonRow(entry: $0, shell: shell) }
                    ForEach(friends, id: \.id) { FriendRow(friend: $0, shell: shell) }
                }
            }
            heading(L("Playlists"), open: $playlistsOpen)
            if playlistsOpen {
                item(route: .playlists, label: L("All Playlists"), symbol: "music.note.list")
                SidebarPlaylists(shell: shell, section: .mine, scrollTop: $scrollTop, scrollHeight: $scrollHeight, contentHeight: $contentHeight)
            }
            // Other people's public playlists. Hidden when there are none, as on a single-user server.
            if shell.library.playlists.contains(where: { $0.section == .shared }) {
                heading(L("Shared||playlists"), open: $sharedOpen)
                if sharedOpen {
                    SidebarPlaylists(shell: shell, section: .shared, scrollTop: $scrollTop, scrollHeight: $scrollHeight, contentHeight: $contentHeight)
                }
            }
        }
        .padding(.horizontal, T.px(12))
        .padding(.top, T.sidebarListTop)
        .padding(.bottom, T.px(12))
        // Songs carried near the top or bottom edge scroll the sidebar, so every playlist can be reached.
        .background(ScrollViewProbe { [songDrag = shell.songDrag] in songDrag.sidebarScroll = $0 })
    }

    private func item(route: Route, label: String, symbol: String) -> some View {
        let on: Bool = {
            if case .search = shell.route, case .browse(genres: false) = route { return true }
            return shell.route == route
        }()
        return SidebarRow(label: label, on: on) {
            Image(systemName: symbol)
                .font(.system(size: T.px(18)))
                .foregroundStyle((on ? T.tEm : T.tNav).color)
                .frame(width: T.px(22))
        } action: { shell.go(route) }
    }

    /// A heading's line, with the title and an optional item at its end.
    private func headingLine(_ title: String, @ViewBuilder end: () -> some View) -> some View {
        HStack {
            Text(title).font(.navia(T.textSM, .semibold)).foregroundStyle(T.tSection.color)
            Spacer()
            end()
        }
        .padding(.top, T.px(18))
        .padding(.bottom, T.px(7))
        .padding(.leading, T.px(14))
        .padding(.trailing, T.px(12))
    }

    private func heading(_ title: String, open: Binding<Bool>) -> some View {
        headingLine(title) {
            Button { open.wrappedValue.toggle() } label: {
                Image(systemName: open.wrappedValue ? "chevron.down" : "chevron.right").font(.system(size: T.px(9), weight: .bold))
                    .foregroundStyle(T.t3.color)
            }
            .buttonStyle(.plain)
            .help(open.wrappedValue ? L("Collapse {section}", ["section": title]) : L("Expand {section}", ["section": title]))
            .accessibilityLabel(open.wrappedValue ? L("Collapse {section}", ["section": title]) : L("Expand {section}", ["section": title]))
        }
    }
}

/// The server can't be reached. Sits at the sidebar's foot with Retry.
private struct OfflineRow: View {
    let shell: ShellModel
    @State private var retrying = false

    var body: some View {
        HStack(spacing: T.px(8)) {
            Image(systemName: "icloud.slash.fill").font(.system(size: T.px(20))).foregroundStyle(T.t2.color)
            VStack(alignment: .leading, spacing: 0) {
                Text(L("Offline")).font(.navia(T.textMD, .medium))
                Text(L("Downloads only")).font(.navia(T.textXS)).foregroundStyle(T.t2.color).lineLimit(1).minimumScaleFactor(0.85)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Button(action: retry) {
                Text(L("Retry")).font(.navia(T.textXS, .medium))
                    .padding(.horizontal, T.px(9))
                    .frame(height: T.px(30))
                    .contentShape(Capsule())
            }
            .buttonStyle(.plain)
            .disabled(retrying)
            .opacity(retrying ? 0.5 : 1)
            .glassPill()
        }
        .padding(.horizontal, T.px(10))
        .padding(.vertical, T.px(10))
        .background(T.row.color, in: RoundedRectangle(cornerRadius: T.rMD, style: .continuous))
    }

    private func retry() {
        retrying = true
        shell.player.retryServer()
        Task {
            let ok = await shell.library.retryConnection()
            retrying = false
            if !ok { shell.showToast(L("Still can't reach the server")) }
        }
    }
}

/// One group's playlist rows, either the user's own or other people's public ones. Dragging a row reorders it
/// within its group, and the order is saved on this device.
private struct SidebarPlaylists: View {
    let shell: ShellModel
    let section: PlaylistSection
    @Binding var scrollTop: CGFloat
    @Binding var scrollHeight: CGFloat
    @Binding var contentHeight: CGFloat

    static let space = "sidebar-content"
    /// Set by the scroll view, so the drag can auto-scroll it.
    @MainActor static var scrollTo: (CGFloat) -> Void = { _ in }

    @State private var saved: [String] = []
    @State private var loadedKey = ""
    @State private var dragged: String?
    @State private var pointer: CGFloat = 0
    @State private var grab: CGFloat = 0
    @State private var listTop: CGFloat = 0
    @State private var ignoreClick = false

    private var rowH: CGFloat { T.px(45) }
    private var members: [PlaylistRef] { shell.library.playlists.filter { $0.section == section } }
    private var ordered: [PlaylistRef] { SidebarOrder.merged(saved: saved, current: members) }
    private var gap: Int { max(0, min(ordered.count, Int(((pointer - listTop - grab) / rowH).rounded()))) }

    var body: some View {
        let list = ordered
        let line = dragged.flatMap { id in
            list.firstIndex { $0.id == id }.flatMap { from in gap == from || gap == from + 1 ? nil : gap }
        }
        VStack(spacing: 0) {
            ForEach(list) { row in
                // The user's own public playlists get a quiet mark, since others on the server can see them.
                SidebarRow(label: row.name, on: shell.route == .playlist(row.id), mark: section == .mine && row.isPublic,
                           download: AnyView(DownloadMark(kind: .playlist, id: row.id, shell: shell))) {
                    PlaylistCover(ref: row, small: true).frame(width: T.px(22), height: T.px(22))
                } action: {
                    guard !ignoreClick else { return }
                    shell.go(.playlist(row.id))
                }
                .opacity(dragged == row.id ? 0.35 : 1)
                .simultaneousGesture(drag(row))
                // Songs dragged here are added.
                .songDropTarget(.playlist(row, place: "sidebar"), shell: shell)
            }
        }
        .overlay(alignment: .topLeading) {
            if let line {
                RoundedRectangle(cornerRadius: 1).fill(shell.theme.accent.color)
                    .frame(height: 2)
                    .padding(.horizontal, T.px(8))
                    .offset(y: CGFloat(line) * rowH - 1)
                    .allowsHitTesting(false)
            }
        }
        .overlay(alignment: .topLeading) {
            if let id = dragged, let row = list.first(where: { $0.id == id }) {
                SidebarRow(label: row.name, on: false) {
                    PlaylistCover(ref: row, small: true).frame(width: T.px(22), height: T.px(22))
                } action: {}
                    .background(T.panel2.color, in: RoundedRectangle(cornerRadius: T.rMD, style: .continuous))
                    .shadow(T.shadowGhost)
                    .offset(y: pointer - listTop - grab)
                    .allowsHitTesting(false)
            }
        }
        .onGeometryChange(for: CGFloat.self) { $0.frame(in: .named(Self.space)).minY } action: { listTop = $0 }
        .task(id: shell.library.accountKey) {
            saved = SidebarOrder.load(account: shell.library.accountKey)
            loadedKey = shell.library.accountKey
        }
        .task(id: dragged) { await autoScroll() }
    }

    private func drag(_ row: PlaylistRef) -> some Gesture {
        DragGesture(minimumDistance: 6, coordinateSpace: .named(Self.space))
            .onChanged { value in
                if dragged == nil {
                    dragged = row.id
                    let index = ordered.firstIndex { $0.id == row.id } ?? 0
                    grab = value.startLocation.y - listTop - CGFloat(index) * rowH
                }
                pointer = value.location.y
            }
            .onEnded { _ in
                drop()
                // The press that ended the drag can still count as a click on the row.
                ignoreClick = true
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { ignoreClick = false }
            }
    }

    private func drop() {
        guard let id = dragged else { return }
        var list = ordered
        if let from = list.firstIndex(where: { $0.id == id }), SidebarOrder.move(&list, from: from, toGap: gap) {
            // The other group keeps its place in the shared list, read fresh in case it moved.
            let account = shell.library.accountKey
            let others = SidebarOrder.merged(saved: SidebarOrder.load(account: account), current: shell.library.playlists.filter { $0.section != section })
            saved = list.map(\.id) + others.map(\.id)
            SidebarOrder.save(saved, account: account)
        }
        dragged = nil
    }

    /// Scrolls while the pointer sits near the top or bottom of the visible list.
    private func autoScroll() async {
        guard dragged != nil else { return }
        let edge = T.px(36), speed = T.px(10)
        while !Task.isCancelled {
            let inView = pointer - scrollTop
            var delta: CGFloat = 0
            if inView < edge { delta = -speed * min(1, (edge - inView) / edge) }
            else if inView > scrollHeight - edge { delta = speed * min(1, (inView - (scrollHeight - edge)) / edge) }
            if delta != 0 {
                let target = max(0, min(max(0, contentHeight - scrollHeight), scrollTop + delta))
                if target != scrollTop {
                    Self.scrollTo(target)
                    // The pointer hasn't moved, but the content under it has.
                    pointer += target - scrollTop
                }
            }
            try? await Task.sleep(for: .milliseconds(16))
        }
    }
}

private struct SidebarRow<Icon: View>: View {
    let label: String
    let on: Bool
    /// A small "public" mark at the row's end.
    var mark = false
    /// A kept playlist's download mark, before the public mark.
    var download: AnyView? = nil
    @ViewBuilder let icon: Icon
    let action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: T.px(12)) {
                icon
                Text(label).font(.navia(T.textMD)).foregroundStyle((on ? T.tEm : T.tNav).color).lineLimit(1).truncationMode(.tail)
                Spacer(minLength: 0)
                HStack(spacing: T.px(8)) {
                    if let download { download }
                    if mark {
                        Image(systemName: "person.2").font(.system(size: T.px(11))).foregroundStyle(T.t3.color).accessibilityLabel(L("Public"))
                    }
                }
            }
            .padding(.horizontal, T.px(14))
            .frame(height: T.px(45))
            .background(RoundedRectangle(cornerRadius: T.rMD, style: .continuous).fill(on ? T.hover.color : hovering || HoverDebug.forced("Sidebar:" + label) ? T.row.color : .clear))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
    }
}
