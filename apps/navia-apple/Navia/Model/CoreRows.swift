import AppKit
import Foundation
import NaviaCoreFFI

/// Artwork lives in the core's disk cache. Rows point at it with `navia-art://cover/<key>` URLs that
/// `ArtworkLoader` resolves here. A key is an album id or an `ar-`/`pl-` cover id.
@MainActor final class CoreArt {
    static let shared = CoreArt()
    nonisolated static let scheme = "navia-art"

    nonisolated static func url(for key: String) -> URL? {
        guard !key.isEmpty, let encoded = key.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) else { return nil }
        return URL(string: "\(scheme)://cover/\(encoded)")
    }

    private(set) var core: NaviaCore?

    /// Clears everything for a new account, since ids are per server.
    func reset(core: NaviaCore?) {
        self.core = core
        paths.removeAll()
        tints.removeAll()
        PlaylistTints.shared.reset()
        pending.removeAll()
        waiting.values.joined().forEach { $0.resume(returning: nil) }
        waiting.removeAll()
        ArtworkLoader.shared.clear()
    }

    /// The core's artwork files were deleted, so paths are asked for again. Tints and decoded images stay.
    func forgetFiles() {
        paths.removeAll()
    }

    /// Dominant color per key, once the core has seen the image.
    private(set) var tints: [String: String] = [:]
    var onTint: (() -> Void)?

    func tint(for url: URL) -> String? { tints[Self.key(of: url)] }

    private nonisolated static func key(of url: URL) -> String { url.path.removingPercentEncoding.map { String($0.dropFirst()) } ?? "" }

    private var paths: [String: URL] = [:]
    private var waiting: [String: [CheckedContinuation<URL?, Never>]] = [:]
    /// Newest last, so on a fast scroll the rows the user stopped on load first.
    private var pending: [String] = []
    private var running = 0

    /// Parallel requests to the core. The core's own limit is gentler still on the server.
    private let concurrency = 8
    /// Past this the oldest requests are dropped. Those rows ask again when they scroll back.
    private let backlog = 256

    private init() {}

    /// The core keeps a few fixed sizes so Navidrome caches each one once.
    private static func size(for pixels: Int) -> UInt32? { [128, 256, 512].first { $0 >= pixels }.map(UInt32.init) }

    func file(for url: URL, pixels: Int) async -> URL? {
        let key = Self.key(of: url)
        let size = Self.size(for: pixels)
        let job = "\(size.map(String.init) ?? "orig")@\(key)"
        if let hit = paths[job] { return hit }
        return await withCheckedContinuation { (cont: CheckedContinuation<URL?, Never>) in
            if waiting[job] != nil {
                waiting[job]?.append(cont)
                return
            }
            waiting[job] = [cont]
            pending.append(job)
            while pending.count > backlog {
                let old = pending.removeFirst()
                waiting.removeValue(forKey: old)?.forEach { $0.resume(returning: nil) }
            }
            pump()
        }
    }

    private func pump() {
        guard let core else {
            pending.removeAll()
            let stale = waiting
            waiting.removeAll()
            stale.values.joined().forEach { $0.resume(returning: nil) }
            return
        }
        while running < concurrency, let job = pending.popLast() {
            running += 1
            let parts = job.split(separator: "@", maxSplits: 1)
            let size = UInt32(parts[0])
            let key = parts.count > 1 ? String(parts[1]) : ""
            Task.detached(priority: .utility) {
                var cover: CoverArtRow?
                do {
                    cover = try await core.cover(key: key, size: size)
                } catch {
                    #if DEBUG
                    NSLog("navia: art failed for \(key): \(error)")
                    #endif
                }
                await MainActor.run { [cover] in self.finished(job, key: key, cover: cover) }
            }
        }
    }

    private func finished(_ job: String, key: String, cover: CoverArtRow?) {
        running -= 1
        var url: URL?
        if let cover {
            url = URL(fileURLWithPath: cover.path)
            paths[job] = url
            let bytes = cover.tintRgb.prefix(3).map { String(format: "%02x", $0) }.joined()
            if tints[key] != bytes {
                tints[key] = bytes
                onTint?()
            }
        }
        waiting.removeValue(forKey: job)?.forEach { $0.resume(returning: url) }
        pump()
    }
}

nonisolated extension SongRow {
    init(core s: NaviaCoreFFI.SongRow) {
        let albumID = s.albumId ?? ""
        self.init(id: s.id, title: s.title, artist: s.artist ?? "", album: s.album ?? "", albumID: albumID,
                  duration: Int(s.durationSeconds ?? 0), genre: s.genre ?? "", year: s.year.map(Int.init), plays: Int(s.playCount ?? 0),
                  isFavorite: s.starred, isDownloaded: false, artworkURL: CoreArt.url(for: albumID), tintHex: nil,
                  trackNumber: s.track.map(Int.init), hasLyrics: false,
                  isHiRes: (s.bitDepth ?? 0) > 16 || (s.samplingRate ?? 0) > 48_000,
                  rowid: s.rowid,
                  credits: s.artists.map { FeaturedArtist(name: $0.name, photoURL: CoreArt.url(for: "ar-\($0.id)"), artistID: $0.id) },
                  disc: s.disc.map(Int.init), isExplicit: s.explicit, added: s.created)
    }
}

nonisolated extension AlbumRow {
    init(core a: NaviaCoreFFI.AlbumRow) {
        self.init(id: a.id, name: a.name, artist: a.artist ?? "", artistID: a.artistId, year: a.year.map(Int.init),
                  songCount: Int(a.songCount ?? 0), duration: Int(a.durationSeconds ?? 0), genre: a.genre ?? "",
                  artworkURL: CoreArt.url(for: a.id), tintHex: nil, blurb: "", isFavorite: a.starred,
                  isExplicit: a.explicit, discTitles: Dictionary(a.discTitles.map { (Int($0.disc), $0.title) }, uniquingKeysWith: { a, _ in a }),
                  credits: a.artists.map { FeaturedArtist(name: $0.name, photoURL: CoreArt.url(for: "ar-\($0.id)"), artistID: $0.id) },
                  edition: a.edition)
    }
}

nonisolated extension ArtistRow {
    init(core a: NaviaCoreFFI.ArtistRow, bio: String = "") {
        self.init(id: a.id, name: a.name, photoURL: CoreArt.url(for: "ar-\(a.id)"), tintHex: nil, bio: bio, isFavorite: a.starred)
    }
}

func residentMemoryBytes() -> UInt64 {
    var info = mach_task_basic_info()
    var count = mach_msg_type_number_t(MemoryLayout<mach_task_basic_info>.size / MemoryLayout<natural_t>.size)
    let result = withUnsafeMutablePointer(to: &info) { $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) { task_info(mach_task_self_, task_flavor_t(MACH_TASK_BASIC_INFO), $0, &count) } }
    return result == KERN_SUCCESS ? UInt64(info.resident_size) : 0
}
