import NaviaCoreFFI
import SwiftUI

/// An album's own page, tinted from its artwork.
struct AlbumPage: View {
    let shell: ShellModel
    let id: String

    var body: some View {
        PageFrame(shell: shell, nav: .icons, edgeFade: true) {
            if let album = shell.library.album(id: id) {
                AlbumDetail(shell: shell, album: album)
            } else {
                PageScroll { EmptyStateView(symbol: "square.stack", title: L("This album isn't in your library")) }
            }
        }
    }
}

/// One album's artwork, title, actions and songs. Used as a page and inside the Albums split view.
struct AlbumDetail: View {
    let shell: ShellModel
    let album: AlbumRow
    var embedded = false
    @State private var holder = Once<any SongsViewModel>()
    @State private var blurbOpen = false
    @Environment(\.pageContext) private var ctx
    @Environment(\.isSnapshot) private var isSnapshot

    var body: some View {
        let songs = holder.get { shell.library.songs(forAlbum: album.id) }
        // Decided here, where the page context is known, because the table measures the header in its own host.
        let header = AlbumHeader(shell: shell, album: album, songs: songs, embedded: embedded, stacked: ctx.cssWidth <= 620, searchWidth: pillSearchWidth(ctx), blurbOpen: $blurbOpen)
            .padding(.leading, ctx.sidebarUnder + ctx.gutter)
            .padding(.trailing, ctx.end + ctx.gutter + ctx.dockUnder)
            .padding(.top, PagePadding.top)
            .padding(.bottom, T.px(32))
        let hosted = header.background(alignment: .top) { if !embedded { PageTint(hex: album.tintHex ?? "2a2b33") } }

        ZStack(alignment: .top) {
            if isSnapshot {
                hosted.frame(maxHeight: .infinity, alignment: .top)
            } else if songs.totalCount == 0 {
                VStack(spacing: 0) {
                    hosted
                    EmptyStateView(title: L("No songs in this album"), compact: true)
                }
                .frame(maxHeight: .infinity, alignment: .top)
            } else {
                SongsTable(songs: songs, shell: shell, revision: songs.revision, editRevision: songs.editRevision,
                           nowPlayingID: shell.player.current?.id, variant: .album, pageHeader: AnyView(hosted),
                           bottomInset: PagePadding.bottom(hasBar: ctx.hasBar), discs: discs(songs),
                           trailing: shell.library.moreBy(album: album).map { .section(AnyView(moreBy($0))) })
            }
            if songs.count == 0 && songs.totalCount > 0 && !isSnapshot {
                EmptyStateView(symbol: "magnifyingglass", title: L("No songs match"),
                               hint: songs.query.isEmpty ? L("No songs on this album are in this filter.") : L("Nothing in this album matches “{query}”.", ["query": songs.query]),
                               compact: true, actionTitle: L("Clear")) { songs.query = ""; songs.filter = .all }
                    .padding(.top, PagePadding.top + T.heroArt + T.px(140))
            }
        }
    }
}

extension AlbumDetail {
    /// "More by <album artist>" after the songs. It's hosted in the table, so it takes the page context and side
    /// room itself.
    fileprivate func moreBy(_ more: MoreBy) -> some View {
        let shared = more.albums.sharedNames
        return Shelf(title: L("More by {name}", ["name": more.artist.name]),
                     more: ShelfMore(title: L("See Discography")) { shell.openAlbums(.artist(more.artist.id, more.section), sort: .year) }) {
            ForEach(more.albums) { other in
                AlbumCard(album: other, byArtist: true, shell: shell, showEdition: shared.contains(other.name.lowercased())) { shell.go(.album(other.id)) }
            }
        }
        .padding(.top, T.sectionGap)
        .padding(.leading, ctx.sidebarUnder)
        .padding(.trailing, ctx.end + ctx.dockUnder)
        .environment(\.pageContext, ctx)
        .foregroundStyle(T.t1.color)
    }

    /// Disc names for the song list's headers, only when the album spans more than one disc.
    fileprivate func discs(_ songs: any SongsViewModel) -> [Int: String]? {
        Set(songs.entireList().compactMap(\.disc)).count > 1 ? album.discTitles : nil
    }
}

private struct AlbumHeader: View {
    let shell: ShellModel
    let album: AlbumRow
    let songs: any SongsViewModel
    let embedded: Bool
    /// The art above the title instead of beside it, for narrow pages.
    let stacked: Bool
    let searchWidth: CGFloat
    @Binding var blurbOpen: Bool

    private var tint: Tint { Tint(hex: album.tintHex) }

    var body: some View {
        let library = shell.library
        let current = library.album(id: album.id) ?? album
        let tracks = songs.entireList()
        VStack(alignment: .leading, spacing: T.px(32)) {
            (stacked ? AnyLayout(VStackLayout(alignment: .leading, spacing: T.px(25))) : AnyLayout(HStackLayout(alignment: .top, spacing: T.px(25)))) {
                ArtworkView(url: album.artworkURL, pixels: 600)
                    .frame(width: T.heroArt, height: T.heroArt)
                    .clipShape(RoundedRectangle(cornerRadius: T.rMD, style: .continuous))
                    .artShadow()
                VStack(alignment: .leading, spacing: 0) {
                    HStack(alignment: .lastTextBaseline, spacing: T.textTitle * 0.27) {
                        Text(album.name).font(.navia(T.textTitle, .bold)).foregroundStyle(T.tEm.color).tracking(-0.024 * T.textTitle).lineSpacing(T.px(2)).lineLimit(2).truncationMode(.tail)
                        if album.isExplicit {
                            ExplicitMark(size: T.textTitle).alignmentGuide(.lastTextBaseline) { $0[.bottom] - T.textTitle * 0.1 }
                        }
                    }
                    artistLine.padding(.top, T.px(14))
                    metaLine.padding(.top, T.px(12))
                    if !album.blurb.isEmpty {
                        ExpandingBlurb(text: album.blurb, isOpen: $blurbOpen).padding(.top, T.px(14))
                    }
                }
                .padding(.top, T.px(20))
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            ActionRow(spacing: T.px(8)) {
                PlayActions(songs: tracks, shell: shell) {
                    IconButton(symbol: "heart", label: L("Favorite"), active: current.isFavorite, inGlass: true) { library.toggleFavorite(album: album.id) }
                    DownloadToggle(kind: .album, id: album.id, name: album.name, hasSongs: !tracks.isEmpty, shell: shell)
                    MenuIconButton(symbol: "ellipsis", label: L("More")) {
                        [MenuEntry(title: L("Add to Queue"), enabled: !tracks.isEmpty) { tracks.forEach { shell.player.addToQueue($0) } },
                         SongMenu.addToPlaylist(tracks, name: album.name, shell: shell),
                         shell.shareEntry(album: current),
                         SongMenu.info(.album(album.id), shell: shell)]
                    }
                }
            } trailing: {
                GlassGroup {
                    SortButton(symbol: "line.3.horizontal.decrease", label: L("Filter songs"), options: SongFilter.allCases, title: \.label, current: songs.filter) { songs.filter = $0 }
                }
                SearchPill(placeholder: L("Search Album"), text: Binding(get: { songs.query }, set: { songs.query = $0 }), accent: shell.theme.accent, width: searchWidth)
            }
        }
        .foregroundStyle(T.t1.color)
    }

    @ViewBuilder private var artistLine: some View {
        if album.credits.filter({ $0.artistID != nil }).count > 1 {
            // Several album artists, each its own link.
            LinkedText(pieces: LinkPiece.artist(album), shell: shell)
                .font(.navia(T.textSubtitle, .semibold))
                .foregroundStyle(tint.fg.color)
        } else if let id = album.artistID {
            Button { shell.go(.artist(id)) } label: {
                HStack(spacing: T.px(5)) {
                    Text(album.artist).lineLimit(1).truncationMode(.tail)
                    Image(systemName: "chevron.right").font(.system(size: T.px(16), weight: .bold))
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .font(.navia(T.textSubtitle, .semibold))
            .foregroundStyle(tint.fg.color)
        } else {
            Text(album.artist).font(.navia(T.textSubtitle, .semibold)).foregroundStyle(tint.fg.color)
        }
    }

    private var metaLine: some View {
        let hiRes = songs.entireList().contains(where: \.isHiRes)
        return HStack(spacing: 0) {
            if !album.genre.isEmpty {
                LinkedText(pieces: [LinkPiece(id: 0, text: album.genre, route: .songs(genre: album.genre))], shell: shell)
                    .fontWeight(.semibold).foregroundStyle(tint.fg.color)
                Text(" • ")
            }
            if let year = album.year { Text("\(String(year)) • ") }
            // The edition as its own meta item, like "Deluxe Edition".
            if let edition = album.edition { Text("\(edition) • ") }
            Text(songsAndLength(album.songCount, seconds: album.duration))
            if let status = shell.library.synced?.status(.album, album.id) { Text(" • " + SyncedDownloads.statusText(status)) }
            if hiRes {
                Text(" • ")
                Image(systemName: "waveform").font(.system(size: T.px(14))).padding(.trailing, T.px(5))
                Text(L("Hi-res Lossless"))
            }
        }
        .font(.navia(T.textMD, .medium))
        .foregroundStyle(tint.fgDim.color)
    }
}

/// A three-line blurb with a plain MORE that appears only when the text is cut off. The last line fades out
/// before the label.
struct ExpandingBlurb: View {
    let text: String
    @Binding var isOpen: Bool
    @State private var fullHeight: CGFloat = 0
    @State private var shownHeight: CGFloat = 0
    @State private var moreWidth: CGFloat = 0

    private var clipped: Bool { fullHeight > shownHeight + 1 }

    var body: some View {
        VStack(alignment: .leading, spacing: T.px(6)) {
            Text(text)
                .font(.navia(T.textMD))
                .lineSpacing(T.px(4))
                .foregroundStyle(T.t2.color)
                .lineLimit(isOpen ? nil : 3)
                .fixedSize(horizontal: false, vertical: true)
                .background(alignment: .top) {
                    Text(text).font(.navia(T.textMD)).lineSpacing(T.px(4))
                        .fixedSize(horizontal: false, vertical: true)
                        .hidden()
                        .onGeometryChange(for: CGFloat.self, of: { $0.size.height }) { fullHeight = $0 }
                }
                .onGeometryChange(for: CGFloat.self, of: { $0.size.height }) { shownHeight = $0 }
                .mask { if clipped && !isOpen { tailFade } else { Rectangle() } }
                .overlay(alignment: .bottomTrailing) {
                    if clipped && !isOpen {
                        moreButton(L("MORE"))
                            .onGeometryChange(for: CGFloat.self, of: { $0.size.width }) { moreWidth = $0 }
                    }
                }
            if isOpen && clipped == false && fullHeight > 0 && shownHeight >= fullHeight {
                moreButton(L("LESS"))
            }
        }
        .frame(maxWidth: T.px(620), alignment: .leading)
    }

    /// Opaque except at the last line's end, which fades out over 40 points and is clear under MORE.
    private var tailFade: some View {
        let line = shownHeight / 3
        return VStack(spacing: 0) {
            Rectangle()
            HStack(spacing: 0) {
                Rectangle()
                LinearGradient(colors: [.black, .clear], startPoint: .leading, endPoint: .trailing).frame(width: T.px(40))
                Color.clear.frame(width: moreWidth + T.px(8))
            }
            .frame(height: line)
        }
    }

    private func moreButton(_ label: String) -> some View {
        Button { isOpen.toggle() } label: {
            Text(label).font(.navia(T.textMD, .semibold)).foregroundStyle(T.t1.color)
                .frame(height: T.px(22)).contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}
