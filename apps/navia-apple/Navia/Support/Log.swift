import Foundation
import os

/// The app's unified log, under the bundle id beside the core's "core" category.
/// Read it with `log show --predicate 'subsystem == "com.davekai.Navia"' --info`.
enum Log {
    static let subsystem = Bundle.main.bundleIdentifier ?? "com.davekai.Navia"
    static let app = Logger(subsystem: subsystem, category: "app")
    static let playback = Logger(subsystem: subsystem, category: "playback")
    static let library = Logger(subsystem: subsystem, category: "library")
}
