// Smoke test for the generated bindings. Makes no sound and needs no server: it calls one
// sync function, lists devices, and checks an async call fails cleanly on a closed port.
import Foundation

print("core version:", naviaCoreVersion())
let devices = try outputDevices()
print("output devices:", devices.count, "default present:", devices.contains { $0.isDefault })
do {
    _ = try await connectSession(serverUrl: "http://127.0.0.1:9", username: "smoke", password: "smoke", readOnly: true, dataDir: "build/smoke-data")
    print("FAIL: connect should not succeed")
    exit(1)
} catch {
    print("connect failed as expected:", error)
}
print("OK")
