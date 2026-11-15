import Foundation

/// The bundle id used to be dev.navia.Navia. At the first launch under the new id, settings saved under the old
/// one are copied over once, without replacing any already set. Check builds (dev.navia.Navia.<name>) keep their own.
nonisolated enum BundleMove {
    static let previousID = "dev.navia.Navia"
    private static let doneKey = "settingsCarriedOver"

    static func carrySettings(into defaults: UserDefaults = .standard, domain: String? = Bundle.main.bundleIdentifier,
                              from previous: String = previousID) {
        guard let domain, !domain.hasPrefix(previous), !defaults.bool(forKey: doneKey) else { return }
        let current = defaults.persistentDomain(forName: domain) ?? [:]
        for (key, value) in defaults.persistentDomain(forName: previous) ?? [:] where current[key] == nil {
            defaults.set(value, forKey: key)
        }
        defaults.set(true, forKey: doneKey)
    }
}
