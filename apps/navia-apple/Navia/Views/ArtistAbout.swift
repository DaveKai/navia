import SwiftUI

/// What an artist's About sheet shows, from what the server has. Nil when there's nothing, which hides the
/// header's info button.
struct ArtistAbout {
    /// How many genres the sheet names.
    static let genresShown = 3

    let facts: [(label: String, value: String)]
    let genres: [String]
    let bio: String

    @MainActor static func of(_ artist: ArtistRow, shell: ShellModel) -> ArtistAbout? {
        var facts: [(label: String, value: String)] = []
        if let a = (shell.library as? CoreLibraryModel)?.core?.artistDetails(id: artist.id) {
            if let n = a.albumCount, n > 0 { facts.append((L("Albums"), fmtInt(Int(n)))) }
            if a.songCount > 0 { facts.append((L("Songs"), fmtInt(Int(a.songCount)))) }
            let roles = a.roles.filter { $0 != "maincredit" }.sorted { GetInfoLayer.roleRank($0) < GetInfoLayer.roleRank($1) }
            if !roles.isEmpty { facts.append((roles.count > 1 ? L("Roles") : L("Role"), roles.map { GetInfoLayer.roleName($0, count: 1) }.joined(separator: ", "))) }
            if a.playCount > 0 { facts.append((L("Plays"), fmtInt(Int(a.playCount)))) }
        }
        var counts: [(genre: String, n: Int)] = []
        for album in shell.library.albums(for: artist) {
            let genre = album.genre.trimmingCharacters(in: .whitespaces)
            guard !genre.isEmpty else { continue }
            if let i = counts.firstIndex(where: { $0.genre == genre }) { counts[i].n += 1 } else { counts.append((genre, 1)) }
        }
        // Most common first. Ties keep the albums' order.
        let genres = counts.enumerated().sorted { ($0.element.n, -$0.offset) > ($1.element.n, -$1.offset) }.prefix(genresShown).map(\.element.genre)
        let bio = artist.bio
        guard !facts.isEmpty || !genres.isEmpty || !bio.isEmpty else { return nil }
        return ArtistAbout(facts: facts, genres: Array(genres), bio: bio)
    }
}

/// The artist's About sheet, with the photo, name, facts, genre chips and biography.
struct ArtistAboutLayer: View {
    let id: String
    let shell: ShellModel

    var body: some View {
        if let artist = shell.library.artist(id: id) {
            let about = ArtistAbout.of(artist, shell: shell)
            DialogCoverSheet(url: artist.photoURL, dismiss: { shell.closeAbout() }) {
                VStack(alignment: .leading, spacing: T.sheetSectionGap) {
                    Text(artist.name)
                        .font(.navia(T.textTitle, .bold)).tracking(-0.026 * T.textTitle)
                        .foregroundStyle(T.tEm.color)
                        .shadow(T.heroTitleShadow)
                        .frame(minHeight: artist.photoURL == nil ? T.controlH : nil)
                        .padding(.leading, artist.photoURL == nil ? CoverSheet.headInset : 0)
                    if let about {
                        if !about.facts.isEmpty {
                            Grid(alignment: .topLeading, horizontalSpacing: T.dialogPad, verticalSpacing: T.dialogGap) {
                                ForEach(Array(stride(from: 0, to: about.facts.count, by: 2)), id: \.self) { i in
                                    GridRow {
                                        fact(about.facts[i])
                                        if i + 1 < about.facts.count { fact(about.facts[i + 1]) } else { Color.clear.frame(height: 1) }
                                    }
                                }
                            }
                        }
                        if !about.genres.isEmpty {
                            VStack(alignment: .leading, spacing: T.sheetLabelGap) {
                                Text(about.genres.count > 1 ? L("Genres") : L("Genre")).font(.navia(T.textSM)).foregroundStyle(T.t2.color)
                                HStack(spacing: T.dialogActionsGap) {
                                    ForEach(about.genres, id: \.self) { genre in
                                        ActionButton(title: genre, small: true) { shell.go(.songs(genre: genre)) }
                                    }
                                }
                            }
                        }
                        if !about.bio.isEmpty {
                            VStack(alignment: .leading, spacing: T.sheetLabelGap) {
                                Text(L("About")).font(.navia(T.textSection, .semibold))
                                Text(about.bio).font(.navia(T.textMD)).foregroundStyle(T.t1.color).lineSpacing(T.px(5)).fixedSize(horizontal: false, vertical: true)
                            }
                        }
                    }
                }
            }
        }
    }

    private func fact(_ f: (label: String, value: String)) -> some View {
        VStack(alignment: .leading, spacing: T.px(2)) {
            Text(f.label).font(.navia(T.textSM)).foregroundStyle(T.t2.color)
            Text(f.value).font(.navia(T.textMD)).foregroundStyle(T.t1.color)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}
