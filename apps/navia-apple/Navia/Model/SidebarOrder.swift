import Foundation

/// The sidebar's playlist order. Navidrome has no playlist order, so it's stored on the device per server and user,
/// keyed by playlist id so renames keep their place.
nonisolated enum SidebarOrder {
    /// `current` reordered, with everything in `saved` that still exists in that order, then anything new.
    static func merged(saved: [String], current: [PlaylistRef]) -> [PlaylistRef] {
        var byID = Dictionary(current.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        var out: [PlaylistRef] = []
        for id in saved {
            if let row = byID.removeValue(forKey: id) { out.append(row) }
        }
        return out + current.filter { byID[$0.id] != nil }
    }

    /// Moves `from` to the gap `gap` (0 is before the first, `count` after the last).
    /// Returns false when that changes nothing.
    @discardableResult static func move<T>(_ items: inout [T], from: Int, toGap gap: Int) -> Bool {
        guard items.indices.contains(from), (0...items.count).contains(gap), gap != from, gap != from + 1 else { return false }
        let item = items.remove(at: from)
        items.insert(item, at: gap > from ? gap - 1 : gap)
        return true
    }

    private static func key(_ account: String) -> String { "sidebarPlaylistOrder.\(account)" }

    static func load(account: String) -> [String] { UserDefaults.standard.stringArray(forKey: key(account)) ?? [] }
    static func save(_ ids: [String], account: String) { UserDefaults.standard.set(ids, forKey: key(account)) }
}
