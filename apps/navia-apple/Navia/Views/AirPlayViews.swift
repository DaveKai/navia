import SwiftUI

/// The AirPlay glyph, used by the Output button, the badge and the chip.
enum AirPlayGlyph {
    static let symbol = "airplay.audio"
}

/// The PIN prompt the first time a receiver needs one. The pairing is remembered. A wrong code makes the receiver
/// show a new one, and a code left too long times out.
struct AirPlayPinLayer: View {
    let shell: ShellModel
    let request: AirPlayModel.PinRequest
    @State private var code = ""
    private let length = 4

    var body: some View {
        let airplay = shell.airplay
        let checking = request.phase == .checking
        let expired = request.phase == .expired
        DialogLayer(title: L("Enter the code shown on {device}", ["device": request.name]), dismiss: { airplay.cancelPin() }) {
            DialogText(text: L("Navia remembers this device, so the code is needed only once."))
            DialogPin(label: L("Code"), value: Binding(get: { expired ? "" : code }, set: { type($0) }), length: length,
                      invalid: request.phase == .wrong, disabled: checking || expired, accent: shell.theme.accent, onSubmit: pair)
            if request.phase == .wrong {
                DialogNote(text: L("That code didn’t work. {device} is showing a new one.", ["device": request.name]), tone: .error)
            }
            if expired { DialogNote(text: L("The code timed out."), tone: .error) }
            DialogActions {
                ActionButton(title: L("Cancel")) { airplay.cancelPin() }
                if expired {
                    ActionButton(title: L("Show a New Code"), kind: .prominent(tint: shell.theme.accent)) { code = ""; airplay.newCode() }
                } else {
                    ActionButton(title: checking ? L("Pairing…") : L("Pair"), kind: .prominent(tint: shell.theme.accent),
                                 enabled: code.count == length && !checking && request.phase != .wrong, busy: checking, action: pair)
                }
            }
        }
    }

    /// After a wrong code, the next digit starts the new one.
    private func type(_ value: String) {
        // The field writes its value back as it appears, so only real typing counts.
        guard value != (request.phase == .expired ? "" : code) else { return }
        if request.phase == .wrong {
            shell.airplay.startNewCode()
            code = String(value.suffix(1))
        } else {
            code = value
        }
    }

    private func pair() {
        guard code.count == length, request.phase == .waiting else { return }
        shell.airplay.submit(code: code)
    }
}

/// Where the music plays on AirPlay: the glyph, in the accent once it plays, and the receiver. `compact` is the
/// name alone, or only the glyph when the bar is narrow. Otherwise it's "Playing on …" under the artist.
struct OutputBadge: View {
    let shell: ShellModel
    var compact = false
    var glyphOnly = false

    var body: some View {
        let airplay = shell.airplay
        if let receiver = airplay.active, let state = receiver.state {
            let text = state == .connecting && !airplay.lost ? L("Connecting to {device}…", ["device": receiver.name])
                : state == .playing ? L("Playing on {device}", ["device": receiver.name]) : receiver.name
            HStack(spacing: T.outputBadgeGap) {
                Image(systemName: AirPlayGlyph.symbol)
                    .font(.system(size: compact ? T.px(12) : T.px(13)))
                    .foregroundStyle(state == .playing ? AnyShapeStyle(shell.theme.accent.color) : AnyShapeStyle(.foreground))
                if !glyphOnly {
                    Text(compact ? receiver.name : text).lineLimit(1).truncationMode(.tail)
                }
            }
            .font(.navia(compact ? T.textXS : T.textSM))
            .frame(maxWidth: compact ? T.outputBadgeMaxW : nil, alignment: .leading)
            .fixedSize(horizontal: compact && glyphOnly, vertical: false)
            .help(text)
            .accessibilityElement(children: .combine)
            .accessibilityLabel(text)
        }
    }
}
