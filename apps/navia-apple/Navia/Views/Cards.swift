import NaviaCoreFFI
import SwiftUI

private struct CardBody<Art: View>: View {
    let width: CGFloat
    let title: String
    var sub: String?
    var centered = false
    var explicit = false
    /// A subtitle with links, used instead of `sub`. A click outside the link text opens the card.
    var subLinks: [LinkPiece]? = nil
    var shell: ShellModel? = nil
    /// A kept playlist's or album's download mark, before the subtitle.
    var download: (kind: CollectionKind, id: String)? = nil
    @ViewBuilder var art: Art

    var body: some View {
        VStack(alignment: centered ? .center : .leading, spacing: 0) {
            art
            TitledText(title: title, explicit: explicit, size: T.textMD, weight: .medium).padding(.top, T.cardTextGap)
            HStack(spacing: T.downloadMarkGap) {
                if let download, let shell { DownloadMark(kind: download.kind, id: download.id, shell: shell) }
                if let subLinks, let shell, !subLinks.isEmpty {
                    LinkedText(pieces: subLinks, shell: shell).font(.navia(T.textMD)).foregroundStyle(T.t2.color)
                } else if let sub, !sub.isEmpty {
                    Text(sub).font(.navia(T.textMD)).foregroundStyle(T.t2.color).lineLimit(3).multilineTextAlignment(centered ? .center : .leading)
                }
            }
        }
        .frame(width: width, alignment: centered ? .center : .leading)
        .contentShape(Rectangle())
    }
}

/// The plain card, lifted on hover with no pressed look.
private struct HoverCard<Label: View>: View {
    let action: () -> Void
    @ViewBuilder let label: (Bool) -> Label
    @State private var hovering = false

    var body: some View {
        Button(action: action) { label(hovering) }
            .buttonStyle(.plain)
            .onHover { hovering = $0 }
    }
}

struct AlbumCard: View {
    let album: AlbumRow
    var width: CGFloat = T.cardW
    var byArtist = false
    var shell: ShellModel? = nil
    /// Another album in the list has the same name, so the subtitle names this one's edition.
    var showEdition = false
    let open: () -> Void

    var body: some View {
        HoverCard(action: open) { hovering in
            let edition = showEdition ? album.edition : nil
            let parts: [String] = byArtist
                ? [album.year.map(String.init), edition, songCount(album.songCount)].compactMap { $0 }
                : [album.artist, album.year.map(String.init), edition].compactMap { $0 }
            // Each artist is its own link. The year stays plain.
            let links: [LinkPiece]? = byArtist ? nil
                : LinkPiece.joined([LinkPiece.artist(album), LinkPiece.plain(album.year.map(String.init) ?? ""), LinkPiece.plain(edition ?? "")], separator: " • ")
            CardBody(width: width, title: album.name, sub: parts.joined(separator: " • "), explicit: album.isExplicit, subLinks: links, shell: shell, download: (.album, album.id)) {
                ArtworkView(url: album.artworkURL, pixels: Int(width * 2))
                    .frame(width: width, height: width)
                    .clipShape(RoundedRectangle(cornerRadius: T.rMD, style: .continuous))
                    .offset(y: hovering ? -T.cardLift : 0)
                    .animation(T.ease(T.med), value: hovering)
            }
        }
    }
}

struct ArtistCard: View {
    let artist: ArtistRow
    var width: CGFloat = T.cardW
    let open: () -> Void

    var body: some View {
        HoverCard(action: open) { _ in
            CardBody(width: width, title: artist.name, centered: true) {
                ArtworkView(url: artist.photoURL, kind: .artist, pixels: Int(width * 2))
                    .frame(width: width, height: width)
                    .clipShape(Circle())
            }
        }
    }
}

struct PlaylistCard: View {
    let playlist: PlaylistRow
    var width: CGFloat = T.gridCard
    var shell: ShellModel? = nil
    let open: () -> Void

    var body: some View {
        HoverCard(action: open) { _ in
            CardBody(width: width, title: playlist.name, sub: playlistSubtitle(playlist), shell: shell, download: (.playlist, playlist.id)) {
                PlaylistCover(playlist: playlist).frame(width: width, height: width)
            }
        }
    }
}

struct MixCard: View {
    let mix: MixInfo
    var width: CGFloat = T.cardW
    let open: () -> Void

    var body: some View {
        HoverCard(action: open) { hovering in
            CardBody(width: width, title: mix.title, sub: mix.artistsLine) {
                PlaylistCover(mix: mix)
                    .frame(width: width, height: width)
                    .offset(y: hovering ? -T.cardLift : 0)
                    .animation(T.ease(T.med), value: hovering)
            }
        }
    }
}

struct CoverLabel: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.navia(T.textXS, .semibold))
            .foregroundStyle(T.tOnArt.color)
            .padding(.horizontal, T.coverLabelPadX)
            .padding(.vertical, T.coverLabelPadY)
            .background(Capsule().fill(T.scrim.color))
            .overlay(Capsule().strokeBorder(T.glassEdge.color, lineWidth: 1))
            .padding(T.coverLabelInset)
    }
}

func playlistSubtitle(_ playlist: PlaylistRow) -> String {
    playlist.isMine || playlist.ownerName.isEmpty ? songCount(playlist.songCount) : "\(songCount(playlist.songCount)) • \(L("By {name}", ["name": playlist.ownerName]))"
}

/// A tile with light text on a tint, such as genre tiles and Artist Radio. It shows `T.panel2` until the tint is
/// known, then the tint darkened enough for light text, as a gradient along `tileFillAngle`.
struct TileFill: View {
    let tint: Tint?
    let width: CGFloat
    let height: CGFloat

    var body: some View {
        if let vivid = tint?.playerVivid {
            let (l, c, h) = OKLCH.from(vivid)
            let shade = OKLCH.rgb(l: l * T.tileFillShade, c: c, h: h)
            // Like CSS's gradient line, through the center at the angle and long enough that the corners land on 0% and 100%.
            let a = T.tileFillAngle * .pi / 180
            let (dx, dy) = (sin(a), -cos(a))
            let half = (abs(width * dx) + abs(height * dy)) / 2
            LinearGradient(colors: [vivid.color, shade.color],
                           startPoint: UnitPoint(x: 0.5 - dx * half / width, y: 0.5 - dy * half / height),
                           endPoint: UnitPoint(x: 0.5 + dx * half / width, y: 0.5 + dy * half / height))
                .transition(.opacity)
        } else {
            T.panel2.color
        }
    }
}

/// One Artist Radio station on Home, with the seed artist and its neighbors that have photos.
struct ArtistStation: Identifiable, Equatable, Sendable {
    let seed: ArtistRow
    let neighbours: [ArtistRow]
    var id: String { seed.id }

    /// "With A, B, C and more", naming up to three neighbors. Empty when there are none.
    var line: String {
        guard !neighbours.isEmpty else { return "" }
        let names = neighbours.prefix(3).map(\.name).joined(separator: ", ")
        return neighbours.count > 3 ? L("With {names} and more", ["names": names]) : L("With {names}", ["names": names])
    }
}

/// An Artist Radio tile on Home, on the tint of the seed's photo. The seed's round photo sits in front of up to two
/// neighbors, which spread on hover. The name shrinks, then truncates, to fit. A click starts the seed's radio.
struct ArtistStationCard: View {
    let station: ArtistStation
    var width: CGFloat = T.cardW
    let play: () -> Void
    @State private var hex: String?

    var body: some View {
        HoverCard(action: play) { hovering in
            VStack(alignment: .leading, spacing: 0) {
                tile(hovering: hovering)
                    .offset(y: hovering ? -T.cardLift : 0)
                    .animation(T.ease(T.med), value: hovering)
                if !station.line.isEmpty {
                    Text(station.line).font(.navia(T.textMD)).foregroundStyle(T.t2.color).lineLimit(2).fixedSize(horizontal: false, vertical: true).padding(.top, T.cardTextGap)
                }
            }
            .frame(width: width, alignment: .leading)
            .contentShape(Rectangle())
        }
        .accessibilityLabel(L("Play {name}", ["name": L("{name} Radio", ["name": station.seed.name])]))
        .task(id: station.seed.photoURL) {
            guard let url = station.seed.photoURL else { return }
            let source = PlaylistCoverSource.image(url)
            if let known = PlaylistTints.shared.cached(source) { hex = known } else { hex = await PlaylistTints.shared.read(source) }
        }
    }

    private func tile(hovering: Bool) -> some View {
        let tint = hex.map { Tint(hex: $0) }
        let face = width * T.stationFace, side = width * T.stationSideFace
        let centreY = width * T.stationFacesY - width / 2
        let shift = width * (hovering ? T.stationSideShiftHover : T.stationSideShift)
        let pad = width * T.stationPad
        return ZStack {
            TileFill(tint: tint, width: width, height: width)
            ForEach(Array(station.neighbours.prefix(2).enumerated()), id: \.element.id) { index, n in
                ArtworkView(url: n.photoURL, kind: .artist, pixels: Int(side * 2))
                    .frame(width: side, height: side)
                    .clipShape(Circle())
                    .offset(x: index == 0 ? -shift : shift, y: centreY)
                    .animation(T.ease(T.med), value: hovering)
            }
            ArtworkView(url: station.seed.photoURL, kind: .artist, pixels: Int(face * 2))
                .frame(width: face, height: face)
                .clipShape(Circle())
                .padding(width * T.stationRing)
                .background(Circle().fill((tint?.playerVivid ?? T.panel2).color))
                .artShadow()
                .offset(y: centreY)
            ViewThatFits(in: .horizontal) {
                name(T.stationName)
                name(T.stationNameLong).truncationMode(.tail)
            }
            .frame(width: width - pad * 2, alignment: .leading)
            .padding(.bottom, pad)
            .frame(width: width, height: width, alignment: .bottom)
        }
        .frame(width: width, height: width)
        .overlay(alignment: .topLeading) { CoverLabel(text: L("Radio")) }
        .clipShape(RoundedRectangle(cornerRadius: T.rMD, style: .continuous))
    }

    private func name(_ share: CGFloat) -> some View {
        Text(station.seed.name).font(.navia(width * share, .bold)).tracking(-0.025 * width * share)
            .foregroundStyle(T.tEm.color).lineLimit(1)
    }
}

struct PlayTile: View {
    let artist: ArtistRow
    let band: String
    let title: String
    let subtitle: String
    var width: CGFloat = T.cardW
    let play: () -> Void

    var body: some View {
        HoverCard(action: play) { hovering in
            let tint = artist.tintHex.flatMap(RGBA.init(hex:)) ?? RGBA(0.33, 0.33, 0.33)
            CardBody(width: width, title: title, sub: subtitle) {
                ZStack(alignment: .top) {
                    tint.color
                    VStack(spacing: 0) {
                        ZStack(alignment: .bottomLeading) {
                            tint.mixed(with: RGBA(hex: "c9c9c9")!, amount: 0.32).color
                            Text(band).font(.system(size: width * 0.136, weight: .semibold)).tracking(-0.025 * width * 0.136)
                                .foregroundStyle(Color(red: 0.067, green: 0.067, blue: 0.067))
                                .padding(.horizontal, width * 0.036)
                                .padding(.bottom, width * 0.03)
                        }
                        .frame(height: width * 0.31)
                        ArtworkView(url: artist.photoURL, kind: .artist, pixels: Int(width * 2), focusY: 0.2)
                    }
                }
                .frame(width: width, height: width)
                .clipShape(RoundedRectangle(cornerRadius: T.rMD, style: .continuous))
                .offset(y: hovering ? -T.cardLift : 0)
                .animation(T.ease(T.med), value: hovering)
            }
        }
        .accessibilityLabel(L("Play {title}", ["title": title]))
    }
}

/// A playlist's cover from the server, or a grid of its album covers when the server has none.
struct PlaylistCover: View {
    var collage: [URL] = []
    var coverURL: URL? = nil
    var small = false
    /// A quiet corner mark for a list the app made, such as "Mix". Hidden at the small size.
    var label: String? = nil

    /// A For You mix, shown as its albums' grid with the "Mix" chip.
    init(mix: MixInfo) {
        collage = mix.collage
        label = L("Mix")
    }

    init(playlist: PlaylistRow, small: Bool = false) {
        collage = playlist.collage
        coverURL = playlist.coverURL
        self.small = small
    }

    init(ref: PlaylistRef, small: Bool = false) {
        collage = ref.collage
        coverURL = ref.coverURL
        self.small = small
    }

    var body: some View {
        GeometryReader { geo in
            let side = geo.size.width
            serverCover(side: side)
                .frame(width: side, height: side, alignment: .topLeading)
                .overlay(alignment: .bottomLeading) {
                    if let label, !small { CoverLabel(text: label) }
                }
                .clipShape(RoundedRectangle(cornerRadius: small ? T.px(5) : T.rMD, style: .continuous))
        }
        .aspectRatio(1, contentMode: .fit)
    }

    @ViewBuilder private func serverCover(side: CGFloat) -> some View {
        if let coverURL {
            ArtworkView(url: coverURL, pixels: Int(side * 2)).frame(width: side, height: side)
        } else if collage.count == 1 {
            ArtworkView(url: collage[0], pixels: Int(side * 2)).frame(width: side, height: side)
        } else if !collage.isEmpty {
            collageGrid(side: side)
        } else {
            ArtworkView(url: nil).frame(width: side, height: side)
        }
    }

    /// Cells in reading order, repeating so none is empty, as Navidrome tiles its own.
    private var cells: [URL] { Self.gridCells(collage) }

    nonisolated static func gridCells<Tile>(_ tiles: [Tile]) -> [Tile] {
        switch tiles.count {
        case 1: [tiles[0], tiles[0], tiles[0], tiles[0]]
        case 2: [tiles[0], tiles[1], tiles[1], tiles[0]]
        case 3: [tiles[0], tiles[1], tiles[2], tiles[0]]
        default: Array(tiles.prefix(4))
        }
    }

    private func collageGrid(side: CGFloat) -> some View {
        VStack(spacing: 0) {
            ForEach(0..<2, id: \.self) { row in
                HStack(spacing: 0) {
                    ForEach(0..<2, id: \.self) { col in
                        ArtworkView(url: cells[row * 2 + col], pixels: Int(side)).frame(width: side / 2, height: side / 2)
                    }
                }
            }
        }
    }
}
