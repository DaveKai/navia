import SwiftUI

/// What the quality badge shows for the current song: the file as the server describes it, and how it reaches the
/// player. Unknown numbers are left out.
nonisolated struct AudioQuality: Equatable, Sendable {
    enum Tier: Sendable { case hiRes, lossless, lossy }
    enum Source: Sendable { case stream, cache, download }

    var format: String
    var tier: Tier
    var kbps: Int?
    var bitDepth: Int?
    var sampleRate: Int?
    var channels: Int?
    /// Where the bytes come from. Nil until the song has opened.
    var source: Source?
    /// The format and bitrate the server transcoded to. Nil for the original file.
    var transcodeFormat: String?
    var transcodeKbps: Int?

    var transcoded: Bool { transcodeFormat != nil }

    /// The badge's text. The bar shortens Hi-Res Lossless to Hi-Res to save room.
    @MainActor func label(short: Bool) -> String {
        if let transcodeFormat { return "\(transcodeFormat) \(transcodeKbps.map(String.init) ?? "")".trimmingCharacters(in: .whitespaces) }
        switch tier {
        case .hiRes: return short ? L("Hi-Res") : L("Hi-Res Lossless")
        case .lossless: return L("Lossless")
        case .lossy: return kbps.map { "\(format) \($0)" } ?? format
        }
    }

    /// Every known fact in one line, like "FLAC · 24-bit · 96 kHz · 2,304 kbps · Stereo".
    @MainActor var detailLine: String {
        var parts = [format]
        if let bitDepth { parts.append(L("{n}-bit", ["n": String(bitDepth)])) }
        if let sampleRate { parts.append(Self.khz(sampleRate)) }
        if let kbps { parts.append(L("{n} kbps", ["n": fmtInt(kbps)])) }
        if let channels { parts.append(Self.channelsLabel(channels)) }
        return parts.joined(separator: " · ")
    }

    /// Formats a sample rate, such as "44.1 kHz".
    @MainActor static func khz(_ hz: Int) -> String {
        let k = Double(hz) / 1000
        return L("{n} kHz", ["n": abs(k - k.rounded()) < 0.05 ? String(Int(k.rounded())) : String(format: "%.1f", k)])
    }

    @MainActor static func channelsLabel(_ n: Int) -> String {
        switch n {
        case 1: L("Mono")
        case 2: L("Stereo")
        case 6: "5.1"
        case 8: "7.1"
        default: L("{n} channels", ["n": fmtInt(n)])
        }
    }

    @MainActor static func sourceLabel(_ source: Source) -> String {
        switch source {
        case .download: L("Downloaded copy")
        case .cache: L("Song cache on this device")
        case .stream: L("Streaming from the server")
        }
    }
}

/// The quiet badge after the time in the dock, bar and expanded player. Hovering briefly or clicking opens the
/// details.
struct QualityBadge: View {
    let shell: ShellModel
    /// The bar's smaller, dimmer one.
    var bar = false
    @State private var probe = WindowFrameProbe()
    @State private var hovering = false
    @State private var hoverTask: Task<Void, Never>?
    @Environment(\.isSnapshot) private var isSnapshot

    static let hoverOpen: Duration = .milliseconds(350)
    static let hoverClose: Duration = .milliseconds(180)

    var body: some View {
        if let quality = shell.player.quality {
            let open = shell.popups.quality != nil
            HStack(spacing: T.px(4)) {
                if quality.transcoded {
                    Image(systemName: "arrow.triangle.2.circlepath").font(.system(size: T.px(9), weight: .bold)).opacity(0.75)
                }
                Text(quality.label(short: bar))
            }
            .font(.navia(T.px(bar ? 11.5 : 12), .medium).monospacedDigit())
            // The dock and expanded player's badge uses the time row's color.
            .foregroundStyle(bar ? AnyShapeStyle(open || hovering ? T.t1.color : T.t2.color) : AnyShapeStyle(.foreground))
            .padding(.horizontal, T.px(bar ? 5 : 6))
            .frame(height: T.px(bar ? 17 : 18))
            .background {
                RoundedRectangle(cornerRadius: T.px(5), style: .continuous).fill(open || hovering ? T.hover.color : .clear)
                RoundedRectangle(cornerRadius: T.px(5), style: .continuous).strokeBorder(.foreground.opacity(quality.transcoded ? 0.18 : 0.34), lineWidth: 1)
            }
            .opacity(bar || open || hovering ? 1 : 0.85)
            .contentShape(Rectangle())
            .background { if !isSnapshot { WindowFrameReader(probe: probe) } }
            .onHover { inside in
                hovering = inside
                guard !(shell.popups.quality?.pinned ?? false) else { return }
                hoverTask?.cancel()
                guard inside else { shell.popups.closeQualitySoon(); return }
                shell.popups.keepQuality()
                hoverTask = Task { @MainActor in
                    try? await Task.sleep(for: Self.hoverOpen)
                    guard !Task.isCancelled, hovering else { return }
                    shell.popups.openQuality(anchor: probe.frame(), pinned: false)
                }
            }
            .onTapGesture {
                hoverTask?.cancel()
                if shell.popups.quality?.pinned == true { shell.popups.closeQuality(unlessPinned: false) } else { shell.popups.openQuality(anchor: probe.frame(), pinned: true) }
            }
            .help(quality.transcoded ? L("{label}, converted by the server. Audio details", ["label": quality.label(short: bar)]) : L("{label}. Audio details", ["label": quality.label(short: bar)]))
            .accessibilityAddTraits(.isButton)
            #if DEBUG
            // For checks, `-naviaForceHover QualityDetails` opens the details once the badge is placed.
            .task {
                guard HoverDebug.forced("QualityDetails") else { return }
                try? await Task.sleep(for: .milliseconds(800))
                shell.popups.openQuality(anchor: probe.frame(), pinned: true)
            }
            #endif
        }
    }
}

/// The badge's details in a float panel centered above the badge. A click outside or Esc closes a pinned one.
/// One opened by hover closes when the pointer leaves both.
struct QualityPopoverLayer: View {
    let shell: ShellModel
    let request: PopupCenter.QualityRequest
    let windowSize: CGSize
    @State private var size: CGSize = .zero

    var body: some View {
        if let quality = shell.player.quality {
            let edge = T.px(10), gap = T.px(8)
            let x = max(edge, min(request.anchor.midX - size.width / 2, windowSize.width - size.width - edge))
            let above = request.anchor.minY - gap - size.height
            let y = above >= edge ? above : request.anchor.maxY + gap
            ZStack(alignment: .topLeading) {
                if request.pinned {
                    Color.clear.contentShape(Rectangle()).onTapGesture { shell.popups.closeQuality(unlessPinned: false) }.accessibilityHidden(true)
                }
                details(quality)
                    .fixedSize()
                    .onGeometryChange(for: CGSize.self, of: { $0.size }) { size = $0 }
                    .offset(x: x, y: y)
                    .opacity(size == .zero ? 0 : 1)
                    .onHover { inside in
                        guard !request.pinned else { return }
                        if inside { shell.popups.keepQuality() } else { shell.popups.closeQualitySoon() }
                    }
            }
            .frame(width: windowSize.width, height: windowSize.height, alignment: .topLeading)
        }
    }

    private func details(_ q: AudioQuality) -> some View {
        let from = q.source.map(AudioQuality.sourceLabel)
        return Grid(alignment: .topLeading, horizontalSpacing: T.px(14), verticalSpacing: T.px(7)) {
            if let format = q.transcodeFormat {
                row(L("Playing"), "\(format) · \(L("{n} kbps", ["n": fmtInt(q.transcodeKbps ?? 0)]))", note: L("Transcoded by the server"))
                row(L("Original"), q.detailLine)
            } else {
                row(L("Playing"), q.detailLine, note: L("The original file"))
            }
            if let from { row(L("From||where a song plays from"), from) }
        }
        .font(.navia(T.textXS).monospacedDigit())
        .foregroundStyle(T.t1.color)
        .padding(.top, T.px(10))
        .padding(.horizontal, T.px(14))
        .padding(.bottom, T.px(11))
        .frame(maxWidth: T.px(400), alignment: .leading)
        .floatPanel()
        .accessibilityElement(children: .combine)
        .accessibilityLabel(L("Audio details"))
    }

    private func row(_ label: String, _ value: String, note: String? = nil) -> some View {
        GridRow {
            Text(label).foregroundStyle(T.t2.color)
            VStack(alignment: .leading, spacing: T.px(1)) {
                Text(value)
                if let note { Text(note).foregroundStyle(T.t2.color) }
            }
        }
    }
}
