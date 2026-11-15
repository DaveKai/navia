import AppKit
import CoreText
import SwiftUI

/// The brand lockup: the H5 mark and the "navia" wordmark in the bundled Mona Sans. Matches GPUI's
/// `ui::brand_lockup` and tools/brand_icons.py.
enum Brand {
    /// The bundled Mona Sans at the wordmark's weight and width.
    static func wordFont(size: CGFloat) -> NSFont {
        guard let url = Bundle.main.url(forResource: "MonaSans", withExtension: "ttf"),
              let base = (CTFontManagerCreateFontDescriptorsFromURL(url as CFURL) as? [CTFontDescriptor])?.first
        else { return .systemFont(ofSize: size, weight: .heavy) }
        let axes: [NSNumber: CGFloat] = [tag("wght"): T.brandWordWeight, tag("wdth"): T.brandWordWidth]
        let descriptor = CTFontDescriptorCreateCopyWithAttributes(base, [kCTFontVariationAttribute: axes] as CFDictionary)
        return CTFontCreateWithFontDescriptor(descriptor, size, nil) as NSFont
    }

    /// A variation axis's four-letter tag as the number Core Text keys axes by.
    private static func tag(_ name: String) -> NSNumber {
        NSNumber(value: name.unicodeScalars.reduce(UInt32(0)) { $0 << 8 | $1.value })
    }
}

/// The H5 mark: x = 50 + R·sin(FX·t + PHASE), y = 50 + R·sin(FY·t) in a 100-unit box, drawn as cubic Béziers whose
/// handles follow the derivative, as `mark_path` in tools/brand_icons.py does. The rect is the ink: the curve's
/// reach plus the stroke.
nonisolated struct NaviaMarkShape: Shape {
    static let radius: CGFloat = 38, fx: CGFloat = 3, fy: CGFloat = 4, phase: CGFloat = .pi / 4, steps = 96
    /// The ink's side in the 100-unit box.
    static var ink: CGFloat { 2 * radius + T.brandMarkStroke }

    func path(in rect: CGRect) -> Path {
        let k = rect.width / Self.ink
        let origin = 50 - Self.ink / 2
        let place = { (x: CGFloat, y: CGFloat) in CGPoint(x: rect.minX + (x - origin) * k, y: rect.minY + (y - origin) * k) }
        let point = { (t: CGFloat) in (50 + Self.radius * sin(Self.fx * t + Self.phase), 50 + Self.radius * sin(Self.fy * t)) }
        let slope = { (t: CGFloat) in (Self.radius * Self.fx * cos(Self.fx * t + Self.phase), Self.radius * Self.fy * cos(Self.fy * t)) }
        let h = 2 * .pi / CGFloat(Self.steps)
        var path = Path()
        let start = point(0)
        path.move(to: place(start.0, start.1))
        for i in 0..<Self.steps {
            let (t0, t1) = (CGFloat(i) * h, CGFloat(i + 1) * h)
            let (p0, p1, d0, d1) = (point(t0), point(t1), slope(t0), slope(t1))
            path.addCurve(to: place(p1.0, p1.1), control1: place(p0.0 + d0.0 * h / 3, p0.1 + d0.1 * h / 3), control2: place(p1.0 - d1.0 * h / 3, p1.1 - d1.1 * h / 3))
        }
        path.closeSubpath()
        return path
    }
}

/// The lockup, `mark` tall. The wordmark and gap follow from it, and the mark centers on the x-height.
struct NaviaLockup: View {
    let mark: CGFloat
    var color: RGBA = T.tEm

    var body: some View {
        let font = Brand.wordFont(size: mark * T.brandWordRatio)
        HStack(alignment: .center, spacing: mark * T.brandGapRatio) {
            NaviaMarkShape()
                .stroke(color.color, style: StrokeStyle(lineWidth: T.brandMarkStroke * mark / NaviaMarkShape.ink, lineJoin: .round))
                .frame(width: mark, height: mark)
            Text(verbatim: "navia")
                .font(Font(font))
                .tracking(T.brandWordTracking * font.pointSize)
                .foregroundStyle(color.color)
                .fixedSize()
                .alignmentGuide(VerticalAlignment.center) { d in d[.firstTextBaseline] - font.xHeight / 2 }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Navia")
    }
}
