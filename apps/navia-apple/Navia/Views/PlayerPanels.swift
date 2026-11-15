import QuartzCore
import SwiftUI

/// Synced lyrics. Lines blur and fade with distance from the current one, and clicking a line seeks to it.
struct LyricsView: View {
    let shell: ShellModel
    var large = false
    /// The window's width in token pixels, which picks the large type size.
    var cssWidth: CGFloat = 0
    @State private var didJump = false
    /// Playback position between the player's reports, so the sweep moves at the display's rate.
    @State private var clock = LyricClock()

    @Environment(\.isSnapshot) private var isSnapshot
    private var player: any PlayerViewModel { shell.player }

    fileprivate func fontSize() -> CGFloat {
        large ? T.px(min(38, max(26, cssWidth * 0.024))) : T.textSection
    }

    /// Room outside the large layout's text column for the blur and the current line's glow. The scroll view and its
    /// mask are this much wider and the text is inset to match. The dock's side padding already gives this room.
    private var blurRoom: CGFloat { large ? T.px(36) : 0 }

    /// Room above the current line. The current line scrolls to this far below the top, and the mask fades the line
    /// before it across this room.
    private var lead: CGFloat { large ? T.px(64) : T.px(36) }

    /// The lyrics' top and bottom fade. Only lines scrolled up past the resting position fade.
    fileprivate func edgeFade(_ height: CGFloat) -> LinearGradient {
        let top = min(0.5, lead / max(1, height))
        return LinearGradient(stops: [.init(color: .clear, location: 0), .init(color: .clear, location: top * T.lyricTopClear),
                                      .init(color: .black, location: top), .init(color: .black, location: 0.88), .init(color: .clear, location: 1)],
                              startPoint: .top, endPoint: .bottom)
    }

    /// Puts a line's zero-height anchor `lead` below the scroll view's top.
    private func leadAnchor(_ height: CGFloat) -> UnitPoint { UnitPoint(x: 0, y: min(0.5, lead / max(1, height))) }

    var body: some View {
        #if DEBUG
        let _ = VisualStats.lyricsBodies += 1
        #endif
        let lines = player.lyrics
        if lines.isEmpty {
            EmptyStateView(symbol: "quote.bubble", title: L("No lyrics for this song"), compact: true)
                .frame(maxHeight: .infinity, alignment: .top)
                .foregroundStyle(T.tOnArt.color)
        } else if !player.lyricsAreSynced(lines) {
            plainLyrics(lines)
        } else {
            GeometryReader { geo in
                ScrollViewReader { proxy in
                    let ms = Int(player.position * 1000)
                    let active = lines.activeIndex(atMs: ms)
                    let activeStart = lines[active].start ?? 0
                    let end = active + 1 < lines.count ? (lines[active + 1].start ?? activeStart + 4000) : activeStart + 4000
                    let span = max(1, min(end - activeStart, 6000))
                    // Real word timing where the server has it, else estimated from the line's span.
                    let sweep = lines[active].sungFraction(atMs: ms) ?? max(0, min(1, Double(ms - activeStart) / Double(span)))
                    // The sweep stops with the clock while hidden and resumes from the current position.
                    let playing = player.isPlaying && !player.isWaitingForServer && shell.windowShown
                    // While paused the sweep is a plain value, so a seek still redraws the line.
                    let timing = playing ? LyricTiming(clock: clock, start: activeStart, span: span, playing: true, line: lines[active].words.isEmpty ? nil : lines[active]) : .fixed(sweep)
                    let lineRange = isSnapshot ? active..<min(active + 9, lines.count) : lines.startIndex..<lines.endIndex
                    Group {
                        if isSnapshot {
                            snapshotBody(lines, lineRange, active: active, sweep: sweep)
                        } else {
                    ScrollView(.vertical) {
                        VStack(alignment: .leading, spacing: large ? T.px(44) : T.px(20)) {
                            ForEach(lines.indices, id: \.self) { index in
                                LyricLineView(text: lines[index].text, distance: abs(index - active), isActive: index == active,
                                              timing: index == active ? timing : nil, fontSize: fontSize(), blurByDistance: shell.lyricBlur) {
                                    if let start = lines[index].start { player.seek(to: Double(start) / 1000) }
                                }
                                .equatable()
                                // A zero-height scroll target at the line's top. Its id has its own type because a bare index would match the
                                // ForEach row, and `scrollTo` would take the row instead.
                                .background(alignment: .top) { Color.clear.frame(height: 0).id(LyricAnchor(index: index)) }
                            }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.top, lead)
                        .padding(.horizontal, large ? blurRoom : T.px(32))
                        .padding(.bottom, geo.size.width * (large ? 0.70 : 0.62))
                    }
                    .scrollIndicators(.never)
                        }
                    }
                    .mask(edgeFade(geo.size.height))
                    .padding(.horizontal, -blurRoom) // Outside the mask, so the mask gets the wider frame too.
                    .onChange(of: player.position, initial: true) { _, position in clock.report(position, playing: playing) }
                    .onChange(of: playing) { _, now in clock.report(player.position, playing: now) }
                    .onChange(of: active) { _, line in
                        // Just shown again, so land on the current line instead of gliding from where it was hidden.
                        if CACurrentMediaTime() - shell.shownAt < 0.5 {
                            #if DEBUG
                            VisualStats.lyricJumps += 1
                            #endif
                            proxy.scrollTo(LyricAnchor(index: line), anchor: leadAnchor(geo.size.height))
                            return
                        }
                        #if DEBUG
                        VisualStats.lyricGlides += 1
                        #endif
                        withAnimation(T.easeInOut(T.med)) { proxy.scrollTo(LyricAnchor(index: line), anchor: leadAnchor(geo.size.height)) }
                    }
                    .onAppear {
                        guard !didJump else { return }
                        didJump = true
                        proxy.scrollTo(LyricAnchor(index: active), anchor: leadAnchor(geo.size.height)) // Jumps into place when the view opens, then glides between lines.
                    }
                }
            }
        }
    }
}

/// The scroll target above a synced lyric line.
private struct LyricAnchor: Hashable { let index: Int }

private extension LyricsView {
    /// Offscreen check images can't draw a scroll view, so they show the current line and those after it.
    func snapshotBody(_ lines: [LyricLine], _ range: Range<Int>, active: Int, sweep: Double) -> some View {
        VStack(alignment: .leading, spacing: large ? T.px(44) : T.px(20)) {
            ForEach(range, id: \.self) { index in
                LyricLineView(text: lines[index].text, distance: abs(index - active), isActive: index == active,
                              timing: index == active ? .fixed(sweep) : nil, fontSize: fontSize(), blurByDistance: shell.lyricBlur, action: {})
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.top, lead)
        .padding(.horizontal, large ? blurRoom : T.px(32))
    }

    /// Unsynced lyrics as plain text, with no blur, highlight or seeking.
    func plainLyrics(_ lines: [LyricLine]) -> some View {
        GeometryReader { geo in
            ScrollView(.vertical) {
                VStack(alignment: .leading, spacing: large ? T.px(18) : T.px(10)) {
                    ForEach(lines.indices, id: \.self) { Text(lines[$0].text).font(.navia(fontSize(), .semibold)).foregroundStyle(T.tOnArt.color).opacity(0.85) }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.top, lead)
                .padding(.horizontal, large ? blurRoom : T.px(32))
                .padding(.bottom, T.px(40))
            }
            .scrollIndicators(.never)
            .mask(edgeFade(geo.size.height))
            .padding(.horizontal, -blurRoom)
        }
    }
}

/// Playback position between the player's reports, which come a few times a second. The sweep reads this every
/// display frame so it moves smoothly.
@MainActor final class LyricClock {
    private var anchor: Double = 0
    private var anchorTime: CFTimeInterval = 0
    private var playing = false

    func report(_ position: Double, playing: Bool) {
        let now = CACurrentMediaTime()
        let predicted = estimate(at: now)
        // Reports jitter a little against the clock. Blend small differences so the sweep never steps back, but take
        // seeks and track changes at once.
        if playing, self.playing, abs(position - predicted) < 0.25 {
            anchor = predicted + (position - predicted) * 0.3
        } else {
            anchor = position
        }
        anchorTime = now
        self.playing = playing
    }

    /// Never runs more than 0.75 s past the last report, so a stalled player can't run the sweep ahead.
    func estimate(at now: CFTimeInterval = CACurrentMediaTime()) -> Double {
        playing ? anchor + min(max(0, now - anchorTime), 0.75) : anchor
    }
}

/// How far the current line's sweep has got, from the clock or a fixed value for check images.
struct LyricTiming: Equatable {
    var clock: LyricClock?
    var start = 0
    var span = 1
    var playing = false
    var fixedProgress: Double? = nil
    /// The line, when it has word timing to follow instead of the span.
    var line: LyricLine? = nil

    static func fixed(_ progress: Double) -> LyricTiming { LyricTiming(fixedProgress: progress) }

    func progress() -> Double {
        if let fixedProgress { return fixedProgress }
        guard let clock else { return 0 }
        let ms = clock.estimate() * 1000
        if let line, let sung = line.sungFraction(atMs: Int(ms)) { return sung }
        return max(0, min(1, (ms - Double(start)) / Double(span)))
    }

    static func == (a: Self, b: Self) -> Bool {
        a.clock === b.clock && a.start == b.start && a.span == b.span && a.playing == b.playing && a.fixedProgress == b.fixedProgress && a.line == b.line
    }
}

/// Draws a lyric line, and on the current line the sweep. The sweep fills wrapped text one visual line at a time,
/// in reading order.
struct LyricSweepRenderer: TextRenderer, Animatable {
    var progress: Double
    var active: Bool

    var animatableData: Double {
        get { progress }
        set { progress = newValue }
    }

    func draw(layout: Text.Layout, in context: inout GraphicsContext) {
        guard active else {
            for line in layout { context.draw(line) }
            return
        }
        let counts = layout.map { line in line.reduce(0) { $0 + $1.count } }
        var filled = progress * Double(max(1, counts.reduce(0, +)))
        for (line, count) in zip(layout, counts) {
            let fraction = count == 0 ? 1 : max(0, min(1, filled / Double(count)))
            filled -= Double(count)
            var base = context
            base.opacity = T.lyricUnsung.a
            base.draw(line)
            guard fraction > 0 else { continue }
            let bounds = line.typographicBounds.rect
            // Generous vertical room so ascenders, descenders and accents aren't clipped.
            let band = bounds.insetBy(dx: -bounds.height, dy: -bounds.height * 0.5)
            let rtl = line.first?.layoutDirection == .rightToLeft
            let soft = max(1, bounds.width * 0.14)
            let edge = rtl ? bounds.maxX - fraction * bounds.width : bounds.minX + fraction * bounds.width
            var lit = context
            lit.clipToLayer { mask in
                let full = rtl ? CGRect(x: edge, y: band.minY, width: band.maxX - edge, height: band.height)
                               : CGRect(x: band.minX, y: band.minY, width: edge - band.minX, height: band.height)
                mask.fill(Path(full), with: .color(.black))
                let fade = CGRect(x: rtl ? edge - soft : edge, y: band.minY, width: soft, height: band.height)
                mask.fill(Path(fade), with: .linearGradient(Gradient(colors: [.black, .clear]),
                                                            startPoint: CGPoint(x: rtl ? edge : edge, y: 0),
                                                            endPoint: CGPoint(x: rtl ? edge - soft : edge + soft, y: 0)))
            }
            lit.draw(line)
        }
    }
}

private struct LyricLineView: View, Equatable {
    let text: String
    let distance: Int
    let isActive: Bool
    /// The current line's sweep timing. Nil on the other lines.
    let timing: LyricTiming?
    let fontSize: CGFloat
    /// The Settings option. Dimming applies either way.
    var blurByDistance = true
    let action: () -> Void
    @State private var hovering = false

    static func == (a: Self, b: Self) -> Bool {
        a.text == b.text && a.distance == b.distance && a.isActive == b.isActive && a.timing == b.timing && a.fontSize == b.fontSize && a.blurByDistance == b.blurByDistance
    }

    private var blur: CGFloat { distance == 0 || hovering || !blurByDistance ? 0 : T.px(min(T.lyricBlurNear + CGFloat(distance) * T.lyricBlurStep, T.lyricBlurMax)) }
    private var opacity: Double { isActive ? 1 : hovering ? T.lyricHover : max(T.lyricDimMin, T.lyricDimNear - Double(distance) * T.lyricDimStep) }

    var body: some View {
        Button(action: action) {
            // Every line has the same structure, so becoming current doesn't rebuild the view. The timeline only runs on
            // the current line while playing.
            TimelineView(.animation(paused: !(isActive && timing?.playing == true))) { _ in
                #if DEBUG
                let _ = isActive ? (VisualStats.lyricFrames += 1) : ()
                #endif
                Text(text)
                    .foregroundStyle(T.tOnArt.color)
                    .textRenderer(LyricSweepRenderer(progress: timing?.progress() ?? 0, active: isActive))
            }
            .shadow(isActive ? T.lyricGlow : T.Shadow(color: .init(1, 1, 1, 0), blur: 0))
            .font(.navia(fontSize, .bold))
            .tracking(-0.01 * fontSize)
            .multilineTextAlignment(.leading)
            .blur(radius: blur)
            .opacity(opacity)
            .animation(T.easeOut(T.med), value: distance)
            .animation(T.easeOut(T.fast), value: hovering)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
    }
}

/// The queue in sections: the song playing, Next in Queue, Next from the source, then Autoplay. Rows move within
/// and between sections by press-and-hold dragging. Rows are built lazily, so a queue of any length is cheap.
struct QueueListView: View {
    let shell: ShellModel
    @State private var selection = RowSelection()
    @State private var drag = RowDrag(rowHeight: T.px(58))
    /// The Deezer card's measured height, so dragging knows where Autoplay's rows sit.
    @State private var deezerCardHeight: CGFloat = 0
    @Environment(\.isSnapshot) private var isSnapshot

    private var player: any PlayerViewModel { shell.player }
    /// Fixed, so dragging knows where the Autoplay rows sit.
    private var autoHeadHeight: CGFloat { T.px(66) }
    /// A section heading after the first, with the gap above it. Fixed, so dragging knows where its rows sit.
    private var sectionHeadHeight: CGFloat { T.queueSectionGap + T.px(39) }

    var body: some View {
        let content = VStack(alignment: .leading, spacing: 0) {
            if let user = player.followingUser {
                // Listening along shows the other person's song, and the listener's own queue waits until following ends.
                heading(L("Now Playing"))
                if let current = player.current { NowPlayingRow(song: current, shell: shell) }
                EmptyStateView(symbol: "headphones", title: L("Listening along with {user}", ["user": user]),
                               hint: player.waitingSong.map { L("Your queue waits. When you stop, it carries on from “{song}”.", ["song": $0.title]) }
                                   ?? L("Songs come from what {user} plays.", ["user": user]), compact: true)
            } else if let station = player.station {
                EmptyStateView(symbol: "dot.radiowaves.left.and.right", title: L("Playing {name}", ["name": station.name]),
                               hint: L("A station has no next song. Your queue carries on when you play something from it."), compact: true)
            } else if let current = player.current {
                heading(L("Now Playing"))
                NowPlayingRow(song: current, shell: shell)
            }
            // While following, the queue waits out of view.
            if player.followingUser == nil {
                if player.queuedCount > 0 {
                    sectionHead(gap: player.current != nil || player.station != nil) {
                        heading(L("Next in Queue"))
                        Spacer(minLength: 0)
                        HStack(spacing: T.queueHeadGap) {
                            headButton(Self.clearSymbol, L("Clear Queue")) { shell.clearQueued(); selection.clear() }
                            headButton("shuffle", L("Shuffle Queue"), disabled: player.queuedCount < 2) { player.shuffleQueued() }
                        }
                    }
                } else if player.upcomingCount > 0 {
                    nextFromHead(gap: player.current != nil || player.station != nil)
                }
                // Only when nothing follows at all, never above Autoplay's songs or while it's finding them.
                if player.upcomingCount == 0 && player.station == nil && autoCount == 0 && player.autoplayShown != .finding {
                    EmptyStateView(symbol: "text.line.first.and.arrowtriangle.forward", title: L("Nothing queued"),
                                   hint: L("Add songs from any menu, or start an album or playlist."), compact: true,
                                   accessory: shell.player.autoplayEnabled ? nil : AnyView(Text(L("Autoplay is off.")).font(.navia(T.textSM)).foregroundStyle(T.t3.color)))
                }
                rows
                if player.autoplayShown == .repeating {
                    autoHeading(L("Off while Repeat is on, because the list never ends."), dimmed: true)
                }
            }
        }
        .padding(.horizontal, T.px(18))
        .padding(.top, T.px(8))
        .padding(.bottom, T.px(24))
        .foregroundStyle(T.tOnArt.color)

        Group {
            if isSnapshot {
                content.frame(maxHeight: .infinity, alignment: .top).clipped()
            } else {
                ScrollView(.vertical) { content }.scrollIndicators(.never)
            }
        }
        .mask(LinearGradient(stops: [.init(color: .black, location: 0.92), .init(color: .clear, location: 1)], startPoint: .top, endPoint: .bottom))
        // Songs dragged in from another list part the rows at the pointer.
        .onDrop(of: [.naviaSongs], delegate: QueueDropDelegate(shell: shell, drag: drag))
        .debugDropFrame(SongDropTarget.queue.key, shell: shell)
        .onAppear { player.setQueueVisible(true) }
        .onDisappear { drag.finish(drop: false); player.setQueueVisible(false) }
        #if DEBUG
        .onAppear { QueueDragDebug.current = drag }
        #endif
    }

    private var autoCount: Int { player.autoplayShown == .on || player.autoplayShown == .finding ? player.autoplayCount : 0 }

    /// A section's heading row, with the section gap above it when another section comes first.
    private func sectionHead(gap: Bool, @ViewBuilder content: () -> some View) -> some View {
        HStack(spacing: T.px(8)) { content() }
            .padding(.trailing, T.px(6))
            .padding(.top, gap ? T.queueSectionGap : 0)
    }

    /// "Next from: <source>" with the source linking to its page, or "Up Next" when it isn't known.
    private func nextFromHead(gap: Bool) -> some View {
        sectionHead(gap: gap) {
            HStack(spacing: T.px(5)) {
                if let source = player.queueSource {
                    Text(L("Next from:")).fixedSize()
                    LinkedText(pieces: [LinkPiece(id: 0, text: source.title, route: source.link)], shell: shell)
                } else {
                    Text(L("Up Next"))
                }
            }
            .font(.navia(T.textLG, .semibold))
            .padding(.horizontal, T.px(8))
            .padding(.top, T.px(6))
            .padding(.bottom, T.px(10))
            Spacer(minLength: 0)
            if player.station == nil {
                HStack(spacing: T.queueHeadGap) {
                    headButton(Self.clearSymbol, player.queueSource.map { L("Clear {name}", ["name": $0.title]) } ?? L("Clear Up Next")) {
                        shell.clearList()
                        selection.clear()
                    }
                    ShuffleButton(shell: shell, size: T.queueHeadIcon, label: L("Shuffle"))
                }
            }
        }
    }

    /// SF Symbols has no broom, so this is the closest match.
    private static let clearSymbol = "eraser.line.dashed"

    /// A one-off action in a queue heading, such as Clear or Shuffle Queue.
    private func headButton(_ symbol: String, _ label: String, disabled: Bool = false, action: @escaping () -> Void) -> some View {
        ModeButton(symbol: symbol, label: label, size: T.queueHeadIcon, on: false, accent: shell.theme.accentSoft, onAccent: shell.theme.onAccent, disabled: disabled, action: action)
    }

    private func heading(_ text: String) -> some View {
        Text(text)
            .font(.navia(T.textLG, .semibold))
            .padding(.horizontal, T.px(8))
            .padding(.top, T.px(6))
            .padding(.bottom, T.px(10))
    }

    /// "Autoplay" with what it's doing under it.
    private func autoHeading(_ note: String?, dimmed: Bool) -> some View {
        VStack(alignment: .leading, spacing: T.px(2)) {
            Text(L("Autoplay")).font(.navia(T.textLG, .semibold)).opacity(dimmed ? 0.6 : 1)
            if let note { Text(note).font(.navia(T.textSM)).opacity(0.6).lineLimit(dimmed ? 2 : 1).truncationMode(.tail) }
        }
        .padding(.horizontal, T.px(8))
        .padding(.top, T.px(18))
        .padding(.bottom, T.px(10))
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// Up Next and Autoplay rows move only while the queue is what's playing. Autoplay songs dragged into Up Next
    /// become queued songs.
    private var canMove: Bool { player.station == nil && (player.upcomingCount > 1 || autoCount > 0) && !isSnapshot }
    /// Any row can be lifted, if only to drag it onto a playlist.
    private var canHold: Bool { player.station == nil && !isSnapshot }

    @ViewBuilder private var rows: some View {
        let count = player.upcomingCount
        let run = min(player.queuedCount, count)
        let autos = autoCount
        let _ = configureDrag(count: count, run: run, autos: autos, gap: autos > 0 ? autoHeadHeight + (asksDeezer ? deezerCardHeight : 0) : 0)
        let stack = Group {
            if isSnapshot {
                VStack(spacing: 0) {
                    ForEach(0..<min(run, 10), id: \.self) { row(at: $0) }
                    splitHead(run: run, count: count)
                    ForEach(run..<min(count, run + 10), id: \.self) { row(at: $0) }
                    autoplayPart(count: count, autos: min(autos, 10))
                }
            } else {
                LazyVStack(spacing: 0) {
                    ForEach(0..<run, id: \.self) { row(at: $0) }
                    splitHead(run: run, count: count)
                    ForEach(run..<count, id: \.self) { row(at: $0) }
                    autoplayPart(count: count, autos: autos)
                }
            }
        }
        stack
            .background(RowDragTracker(drag: drag))
            .background(RowDragSlot(drag: drag))
            .overlay(RowDragOverlay(drag: drag, accent: shell.theme.accent, onAccent: shell.theme.onAccent))
    }

    /// Next from's heading between the sections, when Next in Queue shows above it.
    @ViewBuilder private func splitHead(run: Int, count: Int) -> some View {
        if run > 0 && run < count {
            nextFromHead(gap: true)
                .frame(height: sectionHeadHeight, alignment: .bottom)
                .offset(y: drag.splitShift)
        }
    }

    /// Autoplay's heading and songs. While songs are being found, placeholder rows stand in.
    @ViewBuilder private func autoplayPart(count: Int, autos: Int) -> some View {
        let finding = player.autoplayShown == .finding
        if autos > 0 || finding {
            autoHeading(finding ? L("Finding similar songs") : player.autoplayBasedOn.map { L("Based on {name}", ["name": $0]) }, dimmed: false)
                .frame(height: autoHeadHeight, alignment: .bottom)
                .offset(y: drag.followShift)
            if asksDeezer {
                deezerCard
                    .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { deezerCardHeight = $0 }
                    .offset(y: drag.followShift)
            }
            ForEach(0..<autos, id: \.self) { autoRow(at: $0, upNext: count) }
            if finding {
                ForEach(0..<T.queueFindingRows, id: \.self) { _ in placeholderRow }
            }
        }
    }

    /// Whether Autoplay opens with the Deezer question, for an account that never answered it.
    private var asksDeezer: Bool { shell.deezerChoice == nil && (shell.library as? CoreLibraryModel)?.core != nil }

    /// The Deezer question at the top of Autoplay. Either answer is saved, so it never returns.
    private var deezerCard: some View {
        PromptCard(title: L("Better Autoplay with Deezer?"), symbol: "paperplane", fact: L("Sends artist and song names to Deezer from this computer.")) {
            ActionButton(title: L("Not Now"), small: true, fill: true) { shell.setDeezerChoice(false) }
            ActionButton(title: L("Turn On"), kind: .primary(fill: shell.theme.accent, ink: shell.theme.onAccent), small: true, fill: true) { shell.setDeezerChoice(true) }
        }
        .padding(.bottom, T.promptCardGap)
    }

    /// A placeholder row while Autoplay finds its songs, at a song row's size so the songs replace it without a jump.
    private var placeholderRow: some View {
        HStack(spacing: T.px(11)) {
            RoundedRectangle(cornerRadius: T.rXS, style: .continuous).fill(T.onArtPlaceholder.color).frame(width: T.thumb, height: T.thumb)
            GeometryReader { geo in
                VStack(alignment: .leading, spacing: T.rowPlaceholderGap) {
                    RoundedRectangle(cornerRadius: T.rXS, style: .continuous).fill(T.onArtPlaceholder.color)
                        .frame(width: geo.size.width * T.rowPlaceholderTitleW, height: T.rowPlaceholderLine)
                    RoundedRectangle(cornerRadius: T.rXS, style: .continuous).fill(T.onArtPlaceholder.color)
                        .frame(width: geo.size.width * T.rowPlaceholderArtistW, height: T.rowPlaceholderLine)
                }
                .frame(maxHeight: .infinity, alignment: .center)
            }
        }
        .padding(.horizontal, T.px(8))
        .frame(height: T.px(58))
        .accessibilityHidden(true)
    }

    private func configureDrag(count: Int, run: Int, autos: Int, gap: CGFloat) {
        drag.count = count + autos
        drag.limit = autos > 0 ? count : nil
        drag.limitGap = gap
        // With both sections showing, the heading between them decides where drops go. With neither, dropped Autoplay
        // songs become queued songs.
        drag.split = run > 0 && run < count ? run : nil
        drag.splitGap = sectionHeadHeight
        drag.defaultIntoFirst = run == count
        // With nothing queued, Next from's heading sits above the rows.
        drag.headAbove = run == 0 && count > 0 ? sectionHeadHeight : 0
        let player = player
        let selection = selection
        drag.onMove = { rows, before, intoFirst in
            player.moveInQueue(rows, before: before, queued: intoFirst)
            selection.clear()
        }
        // Dragged past the queue's sides, rows leave as songs that can drop on a playlist.
        let shell = shell
        drag.onLeave = { [weak drag] event, rows in
            let songs = rows.compactMap { index -> SongRow? in
                index < count ? player.upcoming(at: index)?.song : player.autoplay(at: index - count)?.song
            }
            guard let view = drag?.tracker else { return }
            SongDragSource.shared.begin(songs, fromQueue: true, event: event, in: view, shell: shell)
        }
    }

    @ViewBuilder private func row(at offset: Int) -> some View {
        if let entry = player.upcoming(at: offset) {
            let lifted = drag.pressed == offset
            QueueRow(entry: entry, offset: offset, shell: shell, selection: selection, movable: canMove && player.upcomingCount > 1, lifted: lifted, dragging: drag.isActive,
                     hold: canHold ? hold(offset) : nil, menu: { menuEntries(at: $0) },
                     play: { shell.player.jump(toUpcoming: offset) }, remove: removal([offset]), onDelete: { removeSelected() })
                .background { if lifted && drag.rows.count > 1 { StackedCard(tint: shell.player.current?.tintHex) } }
                .opacity(drag.isCarried(offset) ? 0 : 1)
                .offset(y: lifted ? drag.liftedOffset : drag.shift(offset))
                .zIndex(lifted ? 5 : 0)
        }
    }

    @ViewBuilder private func autoRow(at k: Int, upNext: Int) -> some View {
        if let entry = player.autoplay(at: k) {
            let offset = upNext + k
            let lifted = drag.pressed == offset
            QueueRow(entry: entry, offset: offset, shell: shell, selection: selection, movable: canMove, lifted: lifted, dragging: drag.isActive,
                     hold: canHold ? hold(offset) : nil, menu: { menuEntries(at: $0) },
                     play: { shell.player.playAutoplay(k) }, auto: true, remove: removal([offset]),
                     onDelete: { removeSelected() })
                .background { if lifted && drag.rows.count > 1 { StackedCard(tint: shell.player.current?.tintHex) } }
                .opacity(drag.isCarried(offset) ? 0 : 1)
                .offset(y: lifted ? drag.liftedOffset : drag.shift(offset))
                .zIndex(lifted ? 5 : 0)
        }
    }

    /// Removes rows given as offsets in Up Next followed by Autoplay. The playing song is never among them.
    private func remove(_ rows: [Int]) {
        guard player.station == nil else { return }
        let upNext = player.upcomingCount
        let up = rows.filter { $0 < upNext }
        let offered = rows.filter { $0 >= upNext }.map { $0 - upNext }
        // Autoplay's positions don't move when Up Next shrinks, so either order works.
        if !up.isEmpty { player.removeUpcoming(up) }
        if !offered.isEmpty { player.removeAutoplay(offered) }
        selection.clear()
    }

    private func removal(_ rows: [Int]) -> (() -> Void)? {
        player.station == nil && !isSnapshot ? { remove(rows) } : nil
    }

    /// Delete or Backspace takes the selected rows out.
    private func removeSelected() {
        let rows = selection.selected.sorted()
        if !rows.isEmpty { remove(rows) }
    }

    /// Selected rows move together when the pressed one is among them. Otherwise just the pressed one moves.
    private func hold(_ offset: Int) -> RowHold {
        RowHold(deferClick: { [selection] in selection.selected.count > 1 && selection.selected.contains(offset) },
                lift: { [selection, drag] event in
                    let rows = selection.selected.count > 1 && selection.selected.contains(offset) ? selection.selected.sorted() : [offset]
                    if rows.count == 1 { selection.select([offset]) }
                    drag.lift(row: offset, rows: rows, at: event.locationInWindow)
                })
    }

    private func song(at index: Int) -> SongRow? {
        let upNext = player.upcomingCount
        return index < upNext ? player.upcoming(at: index)?.song : player.autoplay(at: index - upNext)?.song
    }

    private func menuEntries(at index: Int) -> [MenuEntry] {
        let canRemove = player.station == nil
        if selection.selected.count > 1, selection.selected.contains(index) {
            let rows = selection.selected.sorted()
            let songs = rows.compactMap { song(at: $0) }
            return SongMenu.several(songs, shell: shell, remove: canRemove ? SongMenu.removeFromQueue(count: rows.count) { remove(rows) } : nil)
        }
        guard let song = song(at: index) else { return [] }
        return SongMenu.single(song, shell: shell, remove: canRemove ? SongMenu.removeFromQueue(count: 1) { remove([index]) } : nil)
    }
}

/// The queue as a drop target for songs from another list. Rows part at the pointer. Songs dropped above the
/// Next from heading go into Next in Queue, and below it they join the list.
struct QueueDropDelegate: DropDelegate {
    let shell: ShellModel
    let drag: RowDrag

    private var takes: Bool { shell.songDrag.active && shell.songDrag.takes(.queue) && shell.player.current != nil && shell.player.station == nil }
    /// The drop location in window coordinates. DropInfo gives it from the top left of the queue's scroll view.
    private func point(_ info: DropInfo) -> NSPoint? {
        guard let scroll = drag.tracker?.enclosingScrollView else { return nil }
        let box = scroll.convert(scroll.bounds, to: nil)
        return NSPoint(x: box.minX + info.location.x, y: box.maxY - info.location.y)
    }

    func validateDrop(info: DropInfo) -> Bool { shell.songDrag.active && info.hasItemsConforming(to: [.naviaSongs]) }
    func dropEntered(info: DropInfo) {
        // The queue's own songs dropped back on it are ignored. Anything else it can't take now is refused.
        if !shell.songDrag.fromQueue { shell.songDrag.enter(.queue, taking: takes) }
        guard takes, let point = point(info) else { return }
        drag.takeIn(count: shell.songDrag.songs.count, at: point)
    }
    func dropUpdated(info: DropInfo) -> DropProposal? {
        guard takes else { return DropProposal(operation: .forbidden) }
        if let point = point(info) { drag.incoming == nil ? drag.takeIn(count: shell.songDrag.songs.count, at: point) : drag.moveIn(point) }
        return DropProposal(operation: .copy)
    }
    func dropExited(info: DropInfo) {
        shell.songDrag.exit(.queue)
        drag.endIn(drop: false)
    }
    func performDrop(info: DropInfo) -> Bool {
        shell.songDrag.exit(.queue)
        guard takes, let place = drag.endIn(drop: true) else { drag.endIn(drop: false); return false }
        let songs = shell.songDrag.songs
        shell.songDrag.end()
        shell.queueSongs(songs, at: place.before, queued: place.intoFirst)
        return true
    }
}

#if DEBUG
/// The queue's drag, for CoreCheck steps that drive it with window points.
@MainActor enum QueueDragDebug {
    static weak var current: RowDrag?
}
#endif

/// The song playing, at the top of the queue. It can't be dragged or clicked, but its artists and album are links.
private struct NowPlayingRow: View {
    let song: SongRow
    let shell: ShellModel

    var body: some View {
        HStack(spacing: T.px(12)) {
            ArtworkView(url: song.artworkURL, pixels: Int(T.thumb * 2))
                .frame(width: T.thumb, height: T.thumb)
                .clipShape(RoundedRectangle(cornerRadius: T.px(6), style: .continuous))
            VStack(alignment: .leading, spacing: 0) {
                TitledText(title: song.title, explicit: song.isExplicit, size: T.textMD, weight: .medium)
                LinkedText(pieces: LinkPiece.artist(song), shell: shell).font(.navia(T.textSM)).opacity(0.7)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Text(clock(Double(song.duration))).font(.navia(T.textSM).monospacedDigit()).opacity(0.65)
        }
        .padding(.horizontal, T.px(8))
        .frame(height: T.px(58))
        // The song's menu, without anything that removes or moves it.
        .rightClickMenu { SongMenu.single(song, shell: shell) }
    }
}

private struct QueueRow: View {
    let entry: QueueEntry
    let offset: Int
    let shell: ShellModel
    let selection: RowSelection
    let movable: Bool
    let lifted: Bool
    let dragging: Bool
    let hold: RowHold?
    let menu: (Int) -> [MenuEntry]
    let play: () -> Void
    /// Dimmed until hovered or selected.
    var auto = false
    /// Shows a remove button in place of the time on hover.
    var remove: (() -> Void)? = nil
    var onDelete: (() -> Void)? = nil
    @State private var hovering = false

    var body: some View {
        // `-naviaForceHover QueueRow:<offset>` draws a row hovered, for checks.
        let hovered = hovering || HoverDebug.forced("QueueRow:\(offset)")
        // On hover, a grip takes the time's place where rows can move. It stays on the lifted row.
        let grip = lifted || (movable && hovered && !dragging)
        let bright = !auto || hovered || lifted || selection.selected.contains(offset)
        SelectableRow(index: offset, selection: selection, height: T.px(58), radius: T.rMD,
                      hoverFill: dragging ? .clear : T.onArtHover.color, selectedFill: T.onArtSelected.color, leadWidth: T.thumb,
                      onPlay: { _ in play() }, menu: menu, onReturn: play,
                      leadPad: T.px(8), trailPad: T.px(8), hold: hold, onDelete: onDelete) {
            ArtworkView(url: entry.song.artworkURL, pixels: Int(T.thumb * 2))
                .frame(width: T.thumb, height: T.thumb)
                .clipShape(RoundedRectangle(cornerRadius: T.px(6), style: .continuous))
                .opacity(bright ? 1 : 0.6)
                .allowsHitTesting(false)
        } text: {
            VStack(alignment: .leading, spacing: 0) {
                TitledText(title: entry.song.title, explicit: entry.song.isExplicit, size: T.textMD, weight: .medium).allowsHitTesting(false)
                LinkedText(pieces: LinkPiece.artist(entry.song), shell: shell) { selection.click(offset, modifiers: $0) }
                    .font(.navia(T.textSM)).opacity(0.7)
            }
            .opacity(bright ? 1 : 0.6)
            .frame(maxWidth: .infinity, alignment: .leading)
            if let remove, hovered, !dragging, !lifted {
                IconButton(symbol: "xmark", label: auto ? L("Remove {name} from Autoplay", ["name": entry.song.title]) : L("Remove {name} from Queue", ["name": entry.song.title]),
                           size: T.px(13), weight: .semibold, width: T.queueRemoveW) { remove() }
                    .help(auto ? L("Remove from Autoplay") : L("Remove from Queue"))
            }
            ZStack(alignment: .trailing) {
                Text(clock(Double(entry.song.duration))).font(.navia(T.textSM).monospacedDigit()).opacity(grip || (remove != nil && hovered && !dragging) ? 0 : bright ? 0.65 : 0.4)
                if grip {
                    Image(systemName: "line.3.horizontal")
                        .font(.system(size: T.px(15), weight: .semibold))
                        .opacity(lifted ? 0.9 : 0.6)
                        .frame(width: T.px(36))
                }
            }
            .allowsHitTesting(false)
        } trailing: {
            EmptyView()
        }
        .background {
            if lifted {
                RoundedRectangle(cornerRadius: T.rMD, style: .continuous)
                    .fill(Tint(hex: shell.player.current?.tintHex).vivid.mixed(with: RGBA(1, 1, 1), amount: 0.93).color)
                    .overlay(RoundedRectangle(cornerRadius: T.rMD, style: .continuous).strokeBorder(T.glassEdge.color, lineWidth: 1))
                    .shadow(T.shadowLift)
            }
        }
        .onHover { hovering = $0 }
    }
}

/// A second card peeking out under the lifted one when several rows move together.
private struct StackedCard: View {
    let tint: String?

    var body: some View {
        RoundedRectangle(cornerRadius: T.rMD, style: .continuous)
            .fill(Tint(hex: tint).vivid.mixed(with: RGBA(1, 1, 1), amount: 0.96).color.opacity(0.88))
            .overlay(RoundedRectangle(cornerRadius: T.rMD, style: .continuous).strokeBorder(T.dragCardUnderEdge.color, lineWidth: 1))
            .padding(.horizontal, T.px(8))
            .offset(y: T.px(6))
    }
}

/// Grows the player out of the dock to fill the window.
nonisolated struct ExpandClip: Shape {
    var progress: Double
    var animatableData: Double { get { progress } set { progress = newValue } }
    func path(in rect: CGRect) -> Path {
        let left = rect.width * 0.72 * (1 - progress)
        let radius = 22 * (1 - progress)
        return Path(roundedRect: CGRect(x: left, y: 0, width: rect.width - left, height: rect.height), cornerRadius: radius, style: .continuous)
    }
}

extension ExpandClip {
    static var animation: Animation? { T.ease(T.med) }
    static var transition: AnyTransition {
        .asymmetric(insertion: .modifier(active: ClipModifier(progress: 0), identity: ClipModifier(progress: 1)),
                    removal: .opacity)
    }
}

private struct ClipModifier: ViewModifier, Animatable {
    var progress: Double
    var animatableData: Double { get { progress } set { progress = newValue } }
    func body(content: Content) -> some View {
        #if DEBUG
        MotionCheck.clipProgress = progress
        #endif
        return content.clipShape(ExpandClip(progress: progress))
    }
}

/// The player filling the whole window, including the sidebar.
struct PlayerExpanded: View {
    let shell: ShellModel
    let windowSize: CGSize
    @Environment(\.isSnapshot) private var isSnapshot
    private var player: any PlayerViewModel { shell.player }

    private var css: CGFloat { windowSize.width / T.scale }
    private var pad: CGFloat { T.px(min(132, max(36, css * 0.0825))) }
    private var gap: CGFloat { T.px(min(193, max(36, css * 0.12))) }
    private var narrow: Bool { css <= 820 }
    private var solo: Bool { shell.shownPlayerView == .artwork }
    /// The text scrim's inputs, the cover at `GlowScrim.grid` and where the controls sit.
    @State private var coverPixels: [UInt8]?
    @State private var controlsRect: CGRect = .zero
    private static let space = "expandedPlayer"

    var body: some View {
        let tint = Tint(hex: player.current?.tintHex)
        ZStack(alignment: .top) {
            LinearGradient(stops: GlowScrim.backdropStops(tint).map { .init(color: $0.color.color, location: $0.location) },
                           startPoint: .topLeading, endPoint: .bottomTrailing)

            ArtworkView(url: player.current?.artworkURL, kind: kind, pixels: 240)
                .frame(width: windowSize.width * 1.4, height: windowSize.height * 1.4)
                .blur(radius: T.playerGlowBlur)
                .saturation(T.playerGlowSaturation)
                .colorMultiply(Color(white: T.playerGlowBrightness))
                .opacity(T.playerGlowOpacity)
                .frame(width: windowSize.width, height: windowSize.height)
                .clipped()
                .allowsHitTesting(false)

            layout

            HStack {
                Spacer()
                IconButton(symbol: "arrow.down.right.and.arrow.up.left", label: L("Collapse player"), size: T.px(20)) { close() }
            }
            .foregroundStyle(T.tOnArt.color)
            .padding(.top, T.px(24))
            .padding(.trailing, T.px(32))

            // Esc closes the player.
            Button("") { close() }.keyboardShortcut(.cancelAction).opacity(0).allowsHitTesting(false).frame(width: 0, height: 0)
        }
        .foregroundStyle(tint.playerFg.color)
        .environment(\.playerDim, tint.playerFgDim.color)
        .environment(\.hoverInk, tint.playerFg.color)
        .coordinateSpace(.named(Self.space))
        .frame(width: windowSize.width, height: windowSize.height)
        .contentShape(Rectangle())
        .task(id: player.current?.artworkURL) {
            coverPixels = nil
            guard let url = player.current?.artworkURL else { return }
            // A cover the core hasn't fetched yet comes back nil at first, so retry with a backoff.
            var delay = 1.0
            while !Task.isCancelled {
                if let image = await ArtworkLoader.shared.load(url, pixels: 240) { coverPixels = GlowScrim.pixels(image); return }
                guard url.scheme == CoreArt.scheme else { return }
                try? await Task.sleep(for: .seconds(delay))
                delay = min(delay * 2, 30)
            }
        }
    }

    /// How dark the scrim behind the controls is. Zero unless the glow under them is too bright for 4.5:1 contrast.
    private func scrimAlpha(_ tint: Tint) -> Double {
        guard let coverPixels else { return 0 }
        return GlowScrim.alpha(peak: GlowScrim.peak(cover: coverPixels, tint: tint, size: windowSize, rect: controlsRect, blur: T.playerGlowBlur), tint: tint)
    }

    private var kind: ArtworkKind { player.station == nil ? .album : .station }
    private func close() { withAnimation(ExpandClip.animation) { shell.expanded = false } }

    private var artColumnWidth: CGFloat {
        let room = windowSize.width - 2 * pad - gap
        return min(T.px(solo ? 420 : 412), max(T.px(260), room))
    }

    @ViewBuilder private var layout: some View {
        if narrow {
            VStack(alignment: .leading, spacing: T.px(18)) {
                playerColumn(artFraction: solo ? 1 : 0.4)
                ViewSwitcher(shell: shell).frame(maxWidth: .infinity).padding(.bottom, T.px(14))
                if !solo { panel.frame(maxHeight: .infinity) }
            }
            .padding(.horizontal, T.px(26))
            .padding(.top, T.px(60))
            .padding(.bottom, T.px(20))
            .frame(maxHeight: .infinity, alignment: .top)
        } else if solo {
            ZStack(alignment: .top) {
                playerColumn(artFraction: 1)
                    .frame(width: artColumnWidth)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                ViewSwitcher(shell: shell)
                    .padding(.top, T.px(24))
            }
        } else {
            HStack(spacing: gap) {
                playerColumn(artFraction: 1).frame(width: artColumnWidth).frame(maxHeight: .infinity)
                ZStack(alignment: .top) {
                    panel
                        .padding(.top, T.px(232))
                        .padding(.bottom, T.px(40))
                    ViewSwitcher(shell: shell)
                        .frame(maxWidth: .infinity)
                        .padding(.top, T.px(152))
                }
            }
            .padding(.horizontal, pad)
        }
    }

    @ViewBuilder private var panel: some View {
        if shell.shownPlayerView == .lyrics {
            LyricsView(shell: shell, large: true, cssWidth: css)
        } else if shell.shownPlayerView == .queue {
            QueueListView(shell: shell)
        }
    }

    private func playerColumn(artFraction: CGFloat) -> some View {
        VStack(spacing: 0) {
            GeometryReader { geo in
                let side = geo.size.width * artFraction
                ArtworkView(url: player.current?.artworkURL, kind: kind)
                    .frame(width: side, height: side)
                    .clipShape(RoundedRectangle(cornerRadius: T.rLG, style: .continuous))
                    .shadow(T.shadowPlayerArt)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            }
            .aspectRatio(1 / artFraction, contentMode: .fit)
            PlayerControls(shell: shell, skipSize: T.px(30), playSize: T.px(44))
                .padding(.top, T.px(22))
                // The text scrim, black and blurred, reaching `glowScrimReach` past the sides and bottom.
                .background(alignment: .top) {
                    let alpha = scrimAlpha(Tint(hex: player.current?.tintHex))
                    if alpha > 0 {
                        RoundedRectangle(cornerRadius: T.glowScrimReach, style: .continuous)
                            .fill(.black.opacity(alpha))
                            .padding(.horizontal, -T.glowScrimReach)
                            .padding(.bottom, -T.glowScrimReach)
                            .blur(radius: T.glowScrimBlur)
                            .allowsHitTesting(false)
                    }
                }
                .onGeometryChange(for: CGRect.self, of: { $0.frame(in: .named(Self.space)) }) { controlsRect = $0 }
        }
    }
}
