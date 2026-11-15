import AppKit
import SwiftUI

/// The account button at the top bar's right: the user's initial in a round glass button, opening a menu with the
/// account, Private Mode, Settings, Change Server and Sign Out. The expanded player has its own, since it covers
/// the top bar.
struct AccountButton: View {
    let shell: ShellModel
    @State private var hovering = false
    @Environment(\.isSnapshot) private var isSnapshot

    private var name: String { shell.activeLogin?.user ?? (CoreMode.useSample ? "sample" : "") }
    private var initial: String { name.trimmingCharacters(in: .whitespaces).first.map { String($0).uppercased() } ?? "?" }

    var body: some View {
        Text(initial)
            .font(.navia(T.textMD, .semibold))
            .foregroundStyle(T.t1.color)
            .frame(width: T.controlH, height: T.controlH)
            .background(HoverFillBackground(on: hovering || HoverDebug.forced(L("Account"))))
            .glassPill()
            .contentShape(Circle())
            .overlay { if !isSnapshot { MenuHitArea(trailing: true, entries: entries) } }
            .onHover { hovering = $0 }
            .help(L("Account"))
            .accessibilityLabel(L("Account"))
            .accessibilityAddTraits(.isButton)
    }

    private func entries() -> [MenuEntry] {
        AccountMenu.entries(shell: shell)
    }
}

enum AccountMenu {
    @MainActor static func entries(shell: ShellModel) -> [MenuEntry] {
        let login = shell.activeLogin
        // A developer override (NAVIDROME_*), sample data or no login has no account to change.
        let switchable = login?.isDeveloperOverride == false
        let reason = login == nil ? L("Not signed in.") : L("Set by NAVIDROME_URL, a developer override.")
        let user = login?.user ?? (CoreMode.useSample ? "sample" : "")
        let host = login?.host ?? (CoreMode.useSample ? "sample data" : "")
        var header = MenuEntry()
        header.view = AccountHeaderView(name: user, host: host)
        var privateMode = MenuEntry(title: L("Private Mode"), checked: shell.player.privateMode) { shell.player.privateMode.toggle() }
        privateMode.isSwitch = true
        var settings = MenuEntry(title: L("Settings…")) { shell.openSettings() }
        settings.shortcut = ","
        return [
            header, .divider, privateMode, .divider, settings,
            MenuEntry(title: L("Change Server"), enabled: switchable, reason: reason) { shell.expanded = false; shell.openChangeServer() },
            MenuEntry(title: L("Sign Out"), enabled: switchable, reason: reason) { shell.requestSignOut() },
        ]
    }
}

/// The menu's first row: the initial in a small circle, the username, and the host.
private final class AccountHeaderView: NSView {
    init(name: String, host: String) {
        super.init(frame: NSRect(x: 0, y: 0, width: 260, height: 48))
        let hosting = NSHostingView(rootView: AccountHeader(name: name, host: host))
        hosting.translatesAutoresizingMaskIntoConstraints = false
        addSubview(hosting)
        NSLayoutConstraint.activate([
            hosting.leadingAnchor.constraint(equalTo: leadingAnchor),
            hosting.trailingAnchor.constraint(equalTo: trailingAnchor),
            hosting.topAnchor.constraint(equalTo: topAnchor),
            hosting.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }

    required init?(coder: NSCoder) { fatalError("not used") }
}

private struct AccountHeader: View {
    let name: String
    let host: String

    var body: some View {
        HStack(spacing: 10) {
            Text(name.trimmingCharacters(in: .whitespaces).first.map { String($0).uppercased() } ?? "?")
                .font(.system(size: 13, weight: .semibold))
                .frame(width: 30, height: 30)
                .background(Circle().fill(Color.primary.opacity(0.12)))
                .overlay(Circle().strokeBorder(Color.primary.opacity(0.16), lineWidth: 1))
            VStack(alignment: .leading, spacing: 1) {
                Text(name).font(.system(size: 13, weight: .semibold)).lineLimit(1).truncationMode(.tail)
                Text(host).font(.system(size: 11)).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 14)
        .frame(width: 260, height: 48, alignment: .leading)
    }
}
