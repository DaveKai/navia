import AppKit
import NaviaCoreFFI
import SwiftUI

/// Share links, for Share…, Edit Link and the Share Links page.
enum ShareRequest: Identifiable {
    case create(ShareTargetRow, ShareSubject)
    case edit(ShareLinkRow)

    var id: String {
        switch self {
        case .create(let target, _): "new-\(target.kind)-\(target.ids.joined(separator: ","))"
        case .edit(let link): "edit-\(link.id)"
        }
    }
}

extension ShellModel {
    /// "Share…" for a target, or the item disabled with the reason when the server has sharing off.
    func shareEntry(_ title: String = L("Share…"), target: ShareTargetRow, subject: @escaping () -> ShareSubject) -> MenuEntry {
        guard let links = library.shareLinks, links.sharingOn else {
            return MenuEntry(title: title, enabled: false, reason: L("Sharing is turned off on this server"), symbol: "square.and.arrow.up")
        }
        return MenuEntry(title: title, symbol: "square.and.arrow.up") { [weak self] in self?.openShare(.create(target, subject())) }
    }

    func shareEntry(songs: [SongRow]) -> MenuEntry {
        let title = songs.count > 1 ? L("Share {n} Songs…", ["n": fmtInt(songs.count)]) : L("Share…")
        return shareEntry(title, target: ShareTargetRow(kind: "song", ids: songs.map(\.id))) { Self.songsSubject(songs) }
    }

    func shareEntry(album: AlbumRow) -> MenuEntry {
        shareEntry(target: ShareTargetRow(kind: "album", ids: [album.id])) { ShareSubject(title: album.name, kind: L("Album"), detail: album.artist, art: album.artworkURL) }
    }

    func shareEntry(artist: ArtistRow) -> MenuEntry {
        shareEntry(target: ShareTargetRow(kind: "artist", ids: [artist.id])) { ShareSubject(title: artist.name, kind: L("Artist"), detail: "", art: artist.photoURL, round: true) }
    }

    func shareEntry(playlist: PlaylistRow) -> MenuEntry {
        shareEntry(target: ShareTargetRow(kind: "playlist", ids: [playlist.id])) { ShareSubject(title: playlist.name, kind: L("Playlist"), detail: "", playlist: playlist) }
    }

    static func songsSubject(_ songs: [SongRow]) -> ShareSubject {
        let first = songs.first
        let art = first.flatMap { CoreArt.url(for: $0.albumID) }
        if songs.count > 1 {
            let oneAlbum = Set(songs.map(\.album)).count == 1, oneArtist = Set(songs.map(\.artist)).count == 1
            return ShareSubject(title: Ln("{count} Song", "{count} Songs", songs.count), kind: L("Songs"), detail: oneAlbum ? first?.album ?? "" : oneArtist ? first?.artist ?? "" : "", art: art)
        }
        return ShareSubject(title: first?.title ?? "", kind: L("Song"), detail: first?.artist ?? "", art: art)
    }

    /// What a link holds, from this library or from the server's entries for items it doesn't know.
    func shareSubject(_ link: ShareLinkRow) -> ShareSubject {
        let id = link.target.ids.first ?? ""
        let entry = link.entries.first
        switch link.target.kind {
        case "playlist":
            let playlist = library.playlist(id: id)
            return ShareSubject(title: playlist?.name ?? entry?.album ?? "", kind: L("Playlist"), detail: "", playlist: playlist)
        case "artist":
            let artist = library.artist(id: id)
            return ShareSubject(title: artist?.name ?? "", kind: L("Artist"), detail: "", art: artist?.photoURL, round: true)
        case "album":
            let album = library.album(id: id)
            let title = link.target.ids.count > 1 ? Ln("{count} Album", "{count} Albums", link.target.ids.count) : album?.name ?? entry?.title ?? ""
            return ShareSubject(title: title, kind: L("Album"), detail: album?.artist ?? entry?.artist ?? "", art: CoreArt.url(for: id))
        default:
            let count = max(link.target.ids.count, link.entries.count)
            let art = entry.flatMap { CoreArt.url(for: $0.albumId) }
            if count > 1 {
                let oneAlbum = Set(link.entries.map(\.album)).count == 1, oneArtist = Set(link.entries.map(\.artist)).count == 1
                return ShareSubject(title: Ln("{count} Song", "{count} Songs", count), kind: L("Songs"), detail: oneAlbum ? entry?.album ?? "" : oneArtist ? entry?.artist ?? "" : "", art: art)
            }
            return ShareSubject(title: entry?.title ?? "", kind: L("Song"), detail: entry?.artist ?? "", art: art)
        }
    }

    func copyShareLink(_ link: ShareLinkRow) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(link.url, forType: .string)
        showToast(L("Link copied"))
    }

    func askDeleteShare(_ link: ShareLinkRow) {
        let title = shareSubject(link).title
        confirm(ConfirmRequest(title: L("Delete this link?"), message: L("The link to “{title}” stops working for everyone who has it.", ["title": title]), confirmTitle: L("Delete")) { [weak self] in
            guard let self, let links = self.library.shareLinks else { return }
            Task { @MainActor in
                if let problem = await links.delete(link.id) { self.showToast(problem) } else { self.showToast(L("Link deleted")) }
            }
        })
    }
}

/// What a link holds, with its cover, title, kind and optionally the link's description.
struct ShareSubjectLine: View {
    let subject: ShareSubject
    var note: String = ""

    var body: some View {
        HStack(spacing: T.px(12)) {
            Group {
                if let playlist = subject.playlist {
                    PlaylistCover(playlist: playlist, small: true)
                } else {
                    ArtworkView(url: subject.art, kind: subject.round ? .artist : .album, pixels: 120)
                }
            }
            .frame(width: T.thumb, height: T.thumb)
            .clipShape(subject.round ? AnyShape(Circle()) : AnyShape(RoundedRectangle(cornerRadius: T.rSM, style: .continuous)))
            VStack(alignment: .leading, spacing: 1) {
                Text(subject.title).font(.navia(T.textMD, .medium)).foregroundStyle(T.t1.color).lineLimit(1)
                Text([subject.kind, subject.detail].filter { !$0.isEmpty }.joined(separator: " • ")).font(.navia(T.textSM)).foregroundStyle(T.t2.color).lineLimit(1)
                if !note.isEmpty {
                    Text(L("“{text}”", ["text": note])).font(.navia(T.textSM)).foregroundStyle(T.t2.color).lineLimit(1)
                }
            }
        }
    }
}

/// The Share… and Edit Link dialog, with a description, an expiry and, where the server allows, downloads.
/// Once created it shows the link with Copy Link and Done.
struct ShareDialogLayer: View {
    let request: ShareRequest
    let shell: ShellModel
    @State private var description: String
    @State private var expiry: ShareLinks.Expiry = .first
    @State private var custom = Date(timeIntervalSinceNow: 30 * 86_400)
    @State private var downloadable: Bool
    @State private var busy = false
    @State private var problem: String?
    @State private var made: ShareLinkRow?

    init(request: ShareRequest, shell: ShellModel) {
        self.request = request
        self.shell = shell
        switch request {
        case .create:
            _description = State(initialValue: "")
            _downloadable = State(initialValue: false)
        case .edit(let link):
            _description = State(initialValue: link.description)
            _downloadable = State(initialValue: link.downloadable ?? false)
        }
    }

    private var links: ShareLinks? { shell.library.shareLinks }
    private var editing: ShareLinkRow? { if case .edit(let link) = request { link } else { nil } }

    private var subject: ShareSubject {
        switch request {
        case .create(_, let subject): subject
        case .edit(let link): shell.shareSubject(link)
        }
    }

    private var title: String {
        switch request {
        case .edit: return L("Edit Link")
        case .create(let target, _):
            switch target.kind {
            case "album": return L("Share Album")
            case "artist": return L("Share Artist")
            case "playlist": return L("Share Playlist")
            default: return target.ids.count > 1 ? L("Share {n} Songs", ["n": fmtInt(target.ids.count)]) : L("Share Song")
            }
        }
    }

    private var expires: UInt64? {
        switch expiry {
        case .first: nil
        case .days(let n): ShareLinks.nowMs() + UInt64(n) * 86_400_000
        case .custom: ShareLinks.endOfDay(custom)
        }
    }

    private var invalid: Bool { expiry == .custom && (expires ?? 0) < ShareLinks.nowMs() }

    var body: some View {
        DialogLayer(title: title, dismiss: { shell.closeShare() }) {
            ShareSubjectLine(subject: subject)
            if let made {
                VStack(alignment: .leading, spacing: T.px(6)) {
                    Text(L("Link")).font(.navia(T.textSM)).foregroundStyle(T.t2.color)
                    Text(made.url).font(.navia(T.textMD)).foregroundStyle(T.t1.color).lineLimit(1).textSelection(.enabled)
                        .padding(.horizontal, T.fieldPad).frame(maxWidth: .infinity, minHeight: T.controlH, alignment: .leading)
                        .background(Capsule().fill(T.glass.color)).overlay(Capsule().strokeBorder(T.glassEdge.color, lineWidth: 1))
                }
                DialogNote(text: note(made))
                DialogActions {
                    ActionButton(title: L("Done")) { shell.closeShare() }
                    ActionButton(title: L("Copy Link"), symbol: "doc.on.doc", kind: .prominent(tint: shell.theme.accent)) { shell.copyShareLink(made) }
                }
            } else {
                form
            }
        }
    }

    @ViewBuilder private var form: some View {
        VStack(alignment: .leading, spacing: T.px(6)) {
            Text(L("Description")).font(.navia(T.textSM)).foregroundStyle(T.t2.color)
            PillTextField(placeholder: L("Optional"), text: $description, accent: shell.theme.accent, onSubmit: submit, onEscape: { shell.closeShare() }, autoFocus: true)
        }
        VStack(alignment: .leading, spacing: T.px(6)) {
            Text(L("Expires")).font(.navia(T.textSM)).foregroundStyle(T.t2.color)
            GlassSelect(shell: shell, label: L("Expires"), value: expiryValue, options: expiryOptions) { value in
                expiry = value == "first" ? .first : value == "custom" ? .custom : .days(ShareLinks.presets.first { $0.id == value }?.days ?? 1)
            }
        }
        if expiry == .custom {
            VStack(alignment: .leading, spacing: T.px(6)) {
                Text(L("Date")).font(.navia(T.textSM)).foregroundStyle(T.t2.color)
                DatePicker(L("Date"), selection: $custom, in: Date(timeIntervalSinceNow: 86_400)..., displayedComponents: .date)
                    .labelsHidden().datePickerStyle(.field)
            }
        }
        if expiry == .first {
            if editing == nil {
                DialogNote(text: L("The server sets the date: a year from now unless its admin chose another."))
            } else if let link = editing, link.expiresMs < ShareLinks.nowMs() {
                DialogNote(text: L("Pick a new date to open the link again."))
            }
        }
        if links?.downloadsOn == true {
            DialogSwitch(label: L("Allow downloads"), hint: L("People with the link can save the songs, not only play them."), isOn: $downloadable, accent: shell.theme.accent)
        }
        if let problem { ErrorText(text: problem) }
        DialogActions {
            ActionButton(title: L("Cancel"), enabled: !busy) { shell.closeShare() }
            ActionButton(title: busy ? L("Saving…") : editing == nil ? L("Create Link") : L("Save"), symbol: editing == nil ? "checkmark" : nil,
                         kind: .prominent(tint: shell.theme.accent), enabled: !busy && !invalid, action: submit)
                .help(invalid ? L("Pick a day after today") : "")
        }
    }

    private var expiryValue: String {
        switch expiry {
        case .first: "first"
        case .custom: "custom"
        case .days(let n): ShareLinks.presets.first { $0.days == n }?.id ?? "first"
        }
    }

    private var expiryOptions: [SelectOption] {
        let first: String
        if let link = editing {
            first = link.expiresMs < ShareLinks.nowMs() ? L("Expired on {date}", ["date": ShareLinks.day(link.expiresMs)]) : ShareLinks.day(link.expiresMs)
        } else {
            first = L("Server default")
        }
        return [SelectOption(value: "first", label: first), SelectOption(value: "1d", label: L("1 day")), SelectOption(value: "1w", label: L("1 week")),
                SelectOption(value: "1m", label: L("1 month")), SelectOption(value: "custom", label: L("Custom date…"))]
    }

    private func note(_ link: ShareLinkRow) -> String {
        let ends = link.expiresMs < ShareLinks.nowMs() ? L("Expired on {date}.", ["date": ShareLinks.day(link.expiresMs)]) : L("Works until {date}.", ["date": ShareLinks.day(link.expiresMs)])
        let what = link.downloadable == true && links?.downloadsOn == true ? L("Anyone with it can play and download.") : L("Anyone with it can play, without signing in.")
        return "\(ends) \(what)"
    }

    private func submit() {
        guard let links, !busy, !invalid else { return }
        busy = true
        problem = nil
        let text = description.trimmingCharacters(in: .whitespaces)
        Task { @MainActor in
            do {
                switch request {
                case .create(let target, _):
                    made = try await links.create(target, description: text, expires: expires, downloadable: downloadable)
                case .edit(let link):
                    try await links.update(link, description: text, expires: expires, downloadable: downloadable)
                    shell.closeShare()
                }
            } catch {
                problem = (error as? LocalizedError)?.errorDescription ?? "\(error)"
            }
            busy = false
        }
    }
}

/// The user's own share links, newest first, with what each shares, its expiry and its visits. An expired link
/// stays listed and can be renewed.
struct SharesPage: View {
    let shell: ShellModel
    @Environment(\.pageContext) private var ctx

    var body: some View {
        PageFrame(shell: shell, nav: .none, title: L("Share Links")) {
            PageScroll { content }
        }
        .task { shell.library.shareLinks?.refresh() }
    }

    @ViewBuilder private var content: some View {
        let links = shell.library.shareLinks
        Group {
            if links?.sharingOn != true {
                EmptyStateView(symbol: "link", title: L("Sharing is turned off on this server"), hint: L("Links can be made once the server’s admin turns sharing on."))
            } else if let list = links?.links {
                if list.isEmpty {
                    EmptyStateView(symbol: "link", title: L("No share links yet"), hint: L("Choose Share… in the menu of a song, album or playlist to make a link anyone can open, without an account."))
                } else {
                    VStack(alignment: .leading, spacing: T.listPageGap) {
                        Text(L("Anyone with one of these links can play what it shares, without an account, until its date.")).font(.navia(T.textMD)).foregroundStyle(T.t2.color)
                        VStack(spacing: 0) { ForEach(list, id: \.id) { ShareRow(shell: shell, link: $0, width: ctx.cssWidth) } }
                    }
                }
            } else if let problem = links?.problem {
                EmptyStateView(symbol: "link", title: L("Share Links"), hint: problem)
            }
        }
        .frame(maxWidth: T.listPageW, alignment: .leading)
        .padding(.horizontal, T.gutter)
        .frame(maxWidth: .infinity)
    }
}

private struct ShareRow: View {
    let shell: ShellModel
    let link: ShareLinkRow
    let width: CGFloat
    @State private var hovering = false

    var body: some View {
        let ended = link.expiresMs < ShareLinks.nowMs()
        let downloads = link.downloadable == true && shell.library.shareLinks?.downloadsOn == true
        HStack(spacing: T.shareSubjectGap) {
            ShareSubjectLine(subject: shell.shareSubject(link), note: link.description).frame(maxWidth: .infinity, alignment: .leading)
            if width > 560 {
                facts(ended ? L("Expired on {date}", ["date": ShareLinks.day(link.expiresMs)]) : L("Until {date}", ["date": ShareLinks.day(link.expiresMs)]),
                      downloads ? L("Downloads allowed") : nil, dim: ended)
                    .frame(width: T.shareEndsW, alignment: .leading)
            }
            if width > 760 {
                facts(link.visits > 0 ? Ln("Opened {count} time", "Opened {count} times", Int(link.visits)) : L("Not opened yet"),
                      link.lastVisitedMs.map { L("Last on {date}", ["date": ShareLinks.day($0)]) }, dim: false)
                    .frame(width: T.shareVisitsW, alignment: .leading)
            }
            HStack(spacing: T.shareActionsGap) {
                IconButton(symbol: "doc.on.doc", label: ended ? L("This link has expired") : L("Copy Link"), disabled: ended) { shell.copyShareLink(link) }
                IconButton(symbol: "pencil", label: L("Edit")) { shell.openShare(.edit(link)) }
                IconButton(symbol: "trash", label: L("Delete")) { shell.askDeleteShare(link) }
            }
            .foregroundStyle(T.t2.color)
        }
        .padding(.horizontal, T.listRowPadX)
        .frame(minHeight: T.rowH)
        .background(RoundedRectangle(cornerRadius: T.rRow, style: .continuous).fill(hovering ? T.row.color : .clear))
        .onHover { hovering = $0 }
    }

    private func facts(_ first: String, _ second: String?, dim: Bool) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(first).foregroundStyle(dim ? T.t3.color : T.t2.color)
            if let second { Text(second).foregroundStyle(T.t3.color) }
        }
        .font(.navia(T.textSM).monospacedDigit())
        .lineLimit(1)
    }
}
