import NaviaCoreFFI
import SwiftUI

/// The download mark. Filled when downloaded, a ring that fills while downloading, and gray while waiting.
/// `queued` is a bare ring and `fetching` a quarter ring.
struct DownloadGlyph: View {
    enum Kind { case done, running, paused, queued, fetching }

    let kind: Kind
    var progress: Double = 0
    var size: CGFloat = T.downloadGlyph
    let theme: Theme
    /// The arrow's color while not done.
    var ink: RGBA = T.t1

    var body: some View {
        Canvas { context, canvasSize in
            // The glyph is drawn on a 24-unit grid.
            let u = canvasSize.width / 24
            let center = CGPoint(x: 12 * u, y: 12 * u)
            if kind == .done {
                context.fill(Path(ellipseIn: CGRect(x: u, y: u, width: 22 * u, height: 22 * u)), with: .color(theme.downloaded.color))
            } else {
                let ring = Path(ellipseIn: CGRect(x: 2 * u, y: 2 * u, width: 20 * u, height: 20 * u))
                context.stroke(ring, with: .color(T.downloadTrack.color), lineWidth: 2 * u)
                let arc: Double = switch kind {
                case .fetching: 0.25
                case .queued: 0
                default: max(0.04, min(1, progress))
                }
                if arc > 0 {
                    var path = Path()
                    path.addArc(center: center, radius: 10 * u, startAngle: .degrees(-90), endAngle: .degrees(-90 + 360 * arc), clockwise: false)
                    context.stroke(path, with: .color((kind == .paused ? T.downloadPaused : theme.downloaded).color), style: StrokeStyle(lineWidth: 2 * u, lineCap: .round))
                }
            }
            var arrow = Path()
            arrow.move(to: CGPoint(x: 12 * u, y: 6.5 * u))
            arrow.addLine(to: CGPoint(x: 12 * u, y: 17 * u))
            arrow.move(to: CGPoint(x: 7.5 * u, y: 12.5 * u))
            arrow.addLine(to: CGPoint(x: 12 * u, y: 17 * u))
            arrow.addLine(to: CGPoint(x: 16.5 * u, y: 12.5 * u))
            let arrowInk: RGBA = switch kind {
            case .done: theme.downloadedInk
            case .queued: T.downloadPaused
            default: ink
            }
            context.stroke(arrow, with: .color(arrowInk.color), style: StrokeStyle(lineWidth: (kind == .done ? 2.4 : 2) * u, lineCap: .round, lineJoin: .round))
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }
}

/// The small mark on a row or card while a collection is downloaded or downloading.
struct DownloadMark: View {
    let kind: CollectionKind
    let id: String
    let shell: ShellModel

    var body: some View {
        if let s = shell.library.synced?.status(kind, id) {
            DownloadGlyph(kind: SyncedDownloads.glyph(s), progress: s.progress, size: T.downloadMark, theme: shell.theme, ink: T.t2)
                .help(SyncedDownloads.statusText(s))
        }
    }
}

/// The Download switch in a playlist's or album's action pill, saved per collection.
/// Turning it off asks first when songs are on the device.
struct DownloadToggle: View {
    let kind: CollectionKind
    let id: String
    let name: String
    let hasSongs: Bool
    let shell: ShellModel

    var body: some View {
        let status = shell.library.synced?.status(kind, id)
        let label = status == nil ? L("Download") : L("Remove Download")
        IconButton(symbol: "arrow.down", label: status.map { "\(SyncedDownloads.statusText($0)). \(label)" } ?? label, active: status != nil, inGlass: true, disabled: !hasSongs,
                   glyph: status.map { AnyView(DownloadGlyph(kind: SyncedDownloads.glyph($0), progress: $0.progress, theme: shell.theme)) }) {
            shell.toggleCollectionDownload(kind, id: id, name: name)
        }
    }
}

/// What turning a collection's Download off asks.
struct StopDownloadRequest {
    let kind: CollectionKind
    let id: String
    let name: String
    let plan: RemovalPlanRow
}

/// "Stop downloading <name>?" with Cancel, Keep Songs when songs would go, then Remove or Stop Syncing.
struct StopDownloadLayer: View {
    let request: StopDownloadRequest
    let shell: ShellModel

    var body: some View {
        let plan = request.plan
        let removes = plan.remove > 0
        DialogLayer(title: L("Stop downloading “{name}”?", ["name": request.name]), dismiss: { shell.closeStopDownload() }) {
            DialogText(text: Self.text(plan))
            if removes { DialogNote(text: L("Keep Songs leaves them on this device as single songs. Either way, it stops staying in step with the server.")) }
            DialogActions {
                ActionButton(title: L("Cancel")) { shell.closeStopDownload() }
                if removes { ActionButton(title: L("Keep Songs")) { shell.stopCollectionDownload(request, keepSongs: true) } }
                ActionButton(title: removes ? L("Remove") : L("Stop Syncing"), kind: .prominent(tint: removes ? T.danger : shell.theme.accent)) {
                    shell.stopCollectionDownload(request, keepSongs: false)
                }
            }
        }
    }

    static func text(_ plan: RemovalPlanRow) -> String {
        var text: String
        if plan.remove > 0 {
            text = Ln("{count} song and {size} will be removed from this device.", "{count} songs and {size} will be removed from this device.", Int(plan.remove), ["size": formatSize(Int64(plan.removeBytes))])
        } else if plan.keep > 0 {
            text = L("Every song in it is also kept by another download, so nothing is removed.")
        } else {
            text = L("None of its songs are on this device yet.")
        }
        if plan.keep > 0, plan.remove > 0 {
            text += " " + Ln("{count} song stays, because another download includes it.", "{count} songs stay, because another download includes them.", Int(plan.keep))
        }
        return text
    }
}
