import Foundation
import Observation

/// Settings > About and Check for Updates. The Mac updater needs Developer ID signing and notarization, so until
/// then no feed is set and a check says so without any network call.
@MainActor @Observable final class AppUpdates {
    static let shared = AppUpdates()

    enum Status: Equatable { case idle, checking, upToDate, ready, notConfigured }

    private(set) var status: Status = .idle

    /// "Check for updates automatically", on by default.
    var checkAutomatically: Bool {
        didSet { UserDefaults.standard.set(checkAutomatically, forKey: "checkUpdates") }
    }

    private init() {
        checkAutomatically = UserDefaults.standard.object(forKey: "checkUpdates") as? Bool ?? true
    }

    /// "0.1.0+543 (f6e8534)" for a test build, "0.1.0" for a release. The version is stamped from the workspace
    /// Cargo.toml by scripts/mac.sh, and the commit from `NaviaCommit`, empty for a release.
    static var versionLabel: String {
        let info = Bundle.main.infoDictionary ?? [:]
        let version = info["CFBundleShortVersionString"] as? String ?? "?"
        let commit = (info["NaviaCommit"] as? String ?? "").trimmingCharacters(in: .whitespaces)
        let build = info["CFBundleVersion"] as? String ?? ""
        if commit.isEmpty { return version }
        return build.isEmpty ? "\(version) (\(commit))" : "\(version)+\(build) (\(commit))"
    }

    /// The release version for the core's User-Agent, never a test build's number, since Navidrome makes a new player
    /// for every new agent.
    nonisolated static var versionForAgent: String {
        Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "?"
    }

    /// The line under "Version", if any.
    var note: String? {
        switch status {
        case .idle: nil
        case .checking: L("Checking…")
        case .upToDate: L("Up to date")
        case .ready: L("A new version is ready.")
        case .notConfigured: L("Updates aren't set up for this build")
        }
    }

    /// No feed yet: says so.
    func check() {
        status = .notConfigured
    }
}
