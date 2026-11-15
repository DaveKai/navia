// Checks the player tint's OKLCH math against reference colors, and the expanded player's scrim rule.
// The app has no test target. Run on the Mac from apps/navia-apple:
//   swiftc -O Navia/Theme/Tokens.swift Navia/Theme/Tint.swift Navia/Theme/GlowScrim.swift tools/TintTests/main.swift -o /tmp/tint-tests && /tmp/tint-tests
import Foundation

/// Reference player colors (OKLCH with a per-channel sRGB clip, as a browser draws it): tint, foreground, dim
/// foreground and background. A blue, a dark navy, a gray and a red cover, and the fallback tint.
let reference: [(String, String, String, String)] = [
    ("29609e", "a0e3ff", "9bd2ff", "055aa9"),
    ("0c0f33", "c4d8ff", "b7c9ff", "1b2050"),
    ("9e9e9e", "dedede", "cecece", "5b5b5b"),
    ("9e2827", "ffbcb1", "ffb2a8", "9e2c2a"),
    ("34506b", "a8e4ff", "9fd3ff", "34597b"),
]

var failures = 0
func check(_ ok: Bool, _ what: String) {
    if !ok { failures += 1; print("FAIL", what) }
}
func hex(_ c: RGBA) -> String { String(format: "%02x%02x%02x", Int((c.r * 255).rounded()), Int((c.g * 255).rounded()), Int((c.b * 255).rounded())) }
func close(_ a: RGBA, _ b: String) -> Bool {
    let e = RGBA(hex: b)!
    return [abs(a.r - e.r), abs(a.g - e.g), abs(a.b - e.b)].allSatisfy { $0 * 255 <= 1.01 }
}

for (tint, fg, dim, vivid) in reference {
    let t = Tint(hex: tint)
    check(close(t.playerFg, fg), "fg \(tint): \(hex(t.playerFg)) vs \(fg)")
    check(close(t.playerFgDim, dim), "dim \(tint): \(hex(t.playerFgDim)) vs \(dim)")
    check(close(t.playerVivid, vivid), "vivid \(tint): \(hex(t.playerVivid)) vs \(vivid)")
    // The glyphs keep 4.5:1. The dimmer step is about 4.3:1 on the lightest backgrounds.
    check(t.playerFg.contrast(against: t.playerVivid) >= 4.5, "contrast \(tint)")
    check(t.playerFgDim.contrast(against: t.playerVivid) >= 4.25, "dim contrast \(tint)")
}
check(hex(Tint(hex: "29609e").playerFg) == "a0e3ff", "the blue cover's glyphs are #a0e3ff")
let grey = Tint(hex: "9e9e9e").playerFg
check(abs(grey.r - grey.g) < 0.004 && abs(grey.g - grey.b) < 0.004, "a grey cover stays grey")

// The scrim: none on a dark backdrop, and enough to bring a bright one to 4.5:1.
let t = Tint(hex: "9e9e9e")
check(GlowScrim.alpha(peak: 0.05, tint: t) == 0, "no scrim when dark")
// A backdrop bright enough to need a scrim, but not past its maximum.
let a = GlowScrim.alpha(peak: 0.25, tint: t)
check(a > 0 && a <= T.glowScrimMax, "scrim in range: \(a)")
// 4.5:1 for the controls' step, 3:1 for the dimmer one.
check((t.playerFg.luminance + 0.05) / (0.25 * pow(1 - a, 2.2) + 0.05) >= 4.5, "scrim reaches 4.5:1 for the controls")
check((t.playerFgDim.luminance + 0.05) / (0.25 * pow(1 - a, 2.2) + 0.05) >= 3.0, "scrim reaches 3:1 for the dim text")
// A white cover over the whole window needs a scrim, and a black one doesn't.
let white = [UInt8](repeating: 255, count: GlowScrim.grid * GlowScrim.grid * 4)
let black = (0..<(GlowScrim.grid * GlowScrim.grid * 4)).map { $0 % 4 == 3 ? UInt8(255) : 0 }
let size = CGSize(width: 1400, height: 875), rect = CGRect(x: 100, y: 500, width: 360, height: 190)
check(GlowScrim.alpha(peak: GlowScrim.peak(cover: white, tint: t, size: size, rect: rect, blur: 96), tint: t) > 0.1, "white cover gets a scrim")
check(GlowScrim.alpha(peak: GlowScrim.peak(cover: black, tint: Tint(hex: "0c0f33"), size: size, rect: rect, blur: 96), tint: Tint(hex: "0c0f33")) == 0, "black cover gets none")

print(failures == 0 ? "tint tests: all passed" : "tint tests: \(failures) failed")
exit(failures == 0 ? 0 : 1)
