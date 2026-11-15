// Window helpers and gated real input for checking a Navia window over SSH.
// Build on the Mac with `swiftc -O -o <path> apps/navia-apple/tools/navia-tool.swift`.
//   windows [pid]            -> pid windowid x y w h layer, for every on-screen window of that pid, or every Navia
//   atpoint <x> <y>          -> every on-screen window at a global point, topmost first: owner pid, layer, alpha
//   idle                     -> seconds since the last HID event
//   pointer                  -> current pointer position
//   run <pid> <batch> [dir]  -> real input from a batch file, gated (below)
//
// So posted input never lands on a Navia someone is using, `run` needs the Mac idle for 4 minutes. Before every
// pointer event it checks that the topmost window at that point belongs to `pid` and that the pointer hasn't been
// moved, and aborts otherwise. Keys and the wheel go to `pid` only, and keys need it to be the active app. The
// pointer is put back at the end. Capture single windows with `screencapture -x -l <windowid>`, never the display.
//
// Batch file, one command per line, `#` for comments:
//   move X Y | click X Y [plain|double|shift|ctrl] | rclick X Y | drag X1 Y1 X2 Y2
//   key return|escape|tab|space|left|right|up|down|delete | type TEXT | wheel X Y LINES [TIMES]
//   capture NAME (this pid's frontmost window, such as an open menu, as <dir>/../NAME.png)
//   cmd STEP (appends STEP to <dir>/cmds for the app's check driver and waits for it to finish)
//   sleep MS
// A window opened with `open -g` isn't active, so have its driver run `activate` first or SwiftUI ignores clicks.
import AppKit
import ApplicationServices
import Foundation

let idleRequired: TimeInterval = 240

/// `HIDIdleTime` in seconds. `CGEventSource`'s any-input query gives wrong answers over SSH.
func idleSeconds() -> TimeInterval {
    let task = Process()
    task.executableURL = URL(fileURLWithPath: "/usr/sbin/ioreg")
    task.arguments = ["-c", "IOHIDSystem"]
    let out = Pipe()
    task.standardOutput = out
    try? task.run()
    let data = out.fileHandleForReading.readDataToEndOfFile()
    task.waitUntilExit()
    let text = String(decoding: data, as: UTF8.self)
    guard let line = text.split(separator: "\n").first(where: { $0.contains("HIDIdleTime") }), let last = line.split(separator: " ").last, let ns = Double(last) else { return 0 }
    return ns / 1_000_000_000
}

func windowsNow() -> [[String: Any]] {
    CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] ?? []
}

func rect(_ w: [String: Any]) -> CGRect {
    let b = w[kCGWindowBounds as String] as? [String: Any] ?? [:]
    return CGRect(x: b["X"] as? Double ?? 0, y: b["Y"] as? Double ?? 0, width: b["Width"] as? Double ?? 0, height: b["Height"] as? Double ?? 0)
}

/// The owner of the topmost window at a point. Skips the Dock's screen-sized input-transparent window and the
/// desktop's layers below zero, since neither takes a click meant for a window.
func topmostOwner(at point: CGPoint) -> Int? {
    let screen = NSScreen.screens.first?.frame.width ?? 0
    for w in windowsNow() where rect(w).contains(point) && (w[kCGWindowAlpha as String] as? Double ?? 1) > 0.01 && (w[kCGWindowLayer as String] as? Int ?? 0) >= 0 {
        if (w[kCGWindowOwnerName as String] as? String) == "Dock", rect(w).width >= screen * 0.9 { continue }
        return w[kCGWindowOwnerPID as String] as? Int
    }
    return nil
}

func die(_ message: String) -> Never {
    print("REFUSED: \(message)")
    exit(1)
}

func parseCommandInt(_ s: String) -> Double {
    guard let v = Double(s) else { die("not a number: \(s)") }
    return v
}

let args = CommandLine.arguments
switch args.count > 1 ? args[1] : "" {
case "windows":
    let pid = args.count > 2 ? Int(args[2]) : nil
    for w in windowsNow() where (pid.map { (w[kCGWindowOwnerPID as String] as? Int) == $0 } ?? ((w[kCGWindowOwnerName as String] as? String) == "Navia")) {
        let r = rect(w)
        print(w[kCGWindowOwnerPID as String] ?? 0, w[kCGWindowNumber as String] ?? 0, Int(r.minX), Int(r.minY), Int(r.width), Int(r.height), "layer", w[kCGWindowLayer as String] ?? 0)
    }
case "atpoint":
    let p = CGPoint(x: parseCommandInt(args[2]), y: parseCommandInt(args[3]))
    for w in windowsNow() where rect(w).contains(p) {
        print(w[kCGWindowOwnerPID as String] ?? 0, "layer", w[kCGWindowLayer as String] ?? 0, "alpha", w[kCGWindowAlpha as String] ?? 1, w[kCGWindowOwnerName as String] ?? "?")
    }
case "idle":
    print(Int(idleSeconds()))
case "pointer":
    let loc = CGEvent(source: nil)?.location ?? .zero
    print(Int(loc.x), Int(loc.y))
case "run":
    guard args.count > 3, let pid = pid_t(args[2]), let text = try? String(contentsOfFile: args[3], encoding: .utf8) else { die("usage: run <pid> <batch> [driver-dir]") }
    let driverDir = args.count > 4 ? args[4] : nil
    // A mouse-sharing server such as Deskflow, Synergy or Barrier forwards input to another computer while the
    // shared cursor is over there, so posted clicks would land on that computer instead.
    let sharing = Process()
    sharing.executableURL = URL(fileURLWithPath: "/usr/bin/pgrep")
    sharing.arguments = ["-if", "deskflow-core server|synergy-core|synergys|barriers"]
    sharing.standardOutput = Pipe()
    try? sharing.run()
    sharing.waitUntilExit()
    guard sharing.terminationStatus != 0 else { die("a mouse-sharing server is running; posted input could reach another computer") }
    let idle = idleSeconds()
    guard idle >= idleRequired else { die("user idle only \(Int(idle)) s, need \(Int(idleRequired))") }
    var pointer = CGEvent(source: nil)?.location ?? .zero
    let restore = pointer
    func finish(_ code: Int32) -> Never {
        CGWarpMouseCursorPosition(restore)
        exit(code)
    }
    func gate(_ p: CGPoint) {
        guard let owner = topmostOwner(at: p), owner == Int(pid) else {
            print("REFUSED: window at \(p) belongs to \(topmostOwner(at: p).map(String.init) ?? "nobody"), not \(pid)")
            finish(1)
        }
    }
    func checkDrift() {
        let now = CGEvent(source: nil)?.location ?? .zero
        if abs(now.x - pointer.x) > 3 || abs(now.y - pointer.y) > 3 {
            print("REFUSED: pointer moved from \(pointer) to \(now), someone is using the Mac")
            finish(1)
        }
    }
    func post(_ type: CGEventType, at p: CGPoint, button: CGMouseButton = .left, flags: CGEventFlags = [], clicks: Int64 = 1) {
        gate(p)
        checkDrift()
        guard let e = CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: p, mouseButton: button) else { return }
        e.flags = flags
        e.setIntegerValueField(.mouseEventClickState, value: clicks)
        e.post(tap: .cghidEventTap)
        pointer = p
    }
    func moveThere(_ p: CGPoint) {
        post(.mouseMoved, at: p)
        usleep(120_000)
    }
    func postKey(_ code: CGKeyCode?, text: String? = nil) {
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pid else {
            print("REFUSED: \(pid) isn't the active app, keys stay unsent")
            finish(1)
        }
        for down in [true, false] {
            guard let e = CGEvent(keyboardEventSource: nil, virtualKey: code ?? 0, keyDown: down) else { continue }
            if let text {
                let scalars = Array(text.utf16)
                e.keyboardSetUnicodeString(stringLength: scalars.count, unicodeString: scalars)
            }
            e.postToPid(pid)
            usleep(20_000)
        }
    }
    let keys: [String: CGKeyCode] = ["return": 36, "tab": 48, "space": 49, "delete": 51, "escape": 53, "left": 123, "right": 124, "down": 125, "up": 126]
    var line = 0
    for raw in text.split(separator: "\n", omittingEmptySubsequences: false) {
        line += 1
        let parts = raw.split(separator: " ", maxSplits: 1).map(String.init)
        guard let verb = parts.first, !verb.hasPrefix("#") else { continue }
        let rest = parts.count > 1 ? parts[1] : ""
        let f = rest.split(separator: " ").map(String.init)
        switch verb {
        case "sleep": usleep(UInt32(parseCommandInt(rest)) * 1000)
        case "move": moveThere(CGPoint(x: parseCommandInt(f[0]), y: parseCommandInt(f[1])))
        case "click":
            let p = CGPoint(x: parseCommandInt(f[0]), y: parseCommandInt(f[1]))
            let kind = f.count > 2 ? f[2] : "plain"
            let flags: CGEventFlags = kind == "shift" ? .maskShift : kind == "ctrl" ? .maskControl : []
            moveThere(p)
            for c in 1...(kind == "double" ? 2 : 1) {
                post(.leftMouseDown, at: p, flags: flags, clicks: Int64(c))
                usleep(70_000)
                post(.leftMouseUp, at: p, flags: flags, clicks: Int64(c))
                usleep(90_000)
            }
        case "rclick":
            let p = CGPoint(x: parseCommandInt(f[0]), y: parseCommandInt(f[1]))
            moveThere(p)
            post(.rightMouseDown, at: p, button: .right)
            usleep(70_000)
            post(.rightMouseUp, at: p, button: .right)
            usleep(90_000)
        case "drag":
            let a = CGPoint(x: parseCommandInt(f[0]), y: parseCommandInt(f[1]))
            let b = CGPoint(x: parseCommandInt(f[2]), y: parseCommandInt(f[3]))
            moveThere(a)
            post(.leftMouseDown, at: a)
            usleep(90_000)
            let steps = 16
            for i in 1...steps {
                let t = Double(i) / Double(steps)
                post(.leftMouseDragged, at: CGPoint(x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t))
                usleep(16_000)
            }
            post(.leftMouseUp, at: b)
            usleep(90_000)
        case "key":
            guard let code = keys[rest] else { die("unknown key \(rest)") }
            postKey(code)
            usleep(60_000)
        case "type":
            for c in rest {
                postKey(nil, text: String(c))
                usleep(UInt32.random(in: 80_000...220_000))
            }
        case "wheel":
            // The wheel can't be posted to the session reliably, and a pid-targeted one can only reach this window.
            // The pointer must already be over the control.
            let p = CGPoint(x: parseCommandInt(f[0]), y: parseCommandInt(f[1]))
            let lines = Int32(parseCommandInt(f[2]))
            let times = f.count > 3 ? Int(f[3])! : 1
            moveThere(p)
            for _ in 0..<times {
                guard let e = CGEvent(scrollWheelEvent2Source: nil, units: .line, wheelCount: 1, wheel1: lines, wheel2: 0, wheel3: 0) else { continue }
                e.location = p
                e.postToPid(pid)
                usleep(60_000)
            }
        case "capture":
            // Only this pid's frontmost window, such as an open menu. `capture NAME` writes <dir>/NAME.png.
            guard let dir = driverDir else { die("capture needs the driver directory argument") }
            guard let id = windowsNow().first(where: { ($0[kCGWindowOwnerPID as String] as? Int) == Int(pid) })?[kCGWindowNumber as String] as? Int else { die("no window") }
            let shot = Process()
            shot.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
            shot.arguments = ["-x", "-l", String(id), "\(dir)/../\(rest).png"]
            try? shot.run()
            shot.waitUntilExit()
        case "cmd":
            guard let dir = driverDir else { die("cmd needs the driver directory argument") }
            let path = dir + "/cmds"
            let wanted = (try? String(contentsOfFile: path, encoding: .utf8))?.split(separator: "\n").count ?? 0
            let handle = FileHandle(forWritingAtPath: path) ?? { FileManager.default.createFile(atPath: path, contents: nil); return FileHandle(forWritingAtPath: path)! }()
            handle.seekToEndOfFile()
            handle.write(Data((rest + "\n").utf8))
            try? handle.close()
            let target = wanted + 1
            for _ in 0..<600 {
                if let done = try? String(contentsOfFile: dir + "/done", encoding: .utf8), let n = Int(done.trimmingCharacters(in: .whitespacesAndNewlines)), n >= target { break }
                usleep(50_000)
            }
        default: die("unknown verb \(verb) on line \(line)")
        }
    }
    print("ok, \(line) lines")
    finish(0)
default:
    print("usage: navia-tool windows [pid] | atpoint x y | idle | pointer | run <pid> <batch> [driver-dir]")
}
