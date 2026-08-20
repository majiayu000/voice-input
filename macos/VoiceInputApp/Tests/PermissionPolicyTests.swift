import XCTest
@testable import VoiceInputApp

final class PermissionPolicyTests: XCTestCase {
    func testPermissionSnapshotDecodesHelperContract() throws {
        let payload = #"""
        {
          "schema_version": 1,
          "subject_executable": "/tmp/Voice Input Runtime.app/Contents/MacOS/voice-input",
          "microphone": "denied",
          "accessibility": "authorized",
          "input_monitoring": "not_determined"
        }
        """#.data(using: .utf8)!

        let snapshot = try JSONDecoder.voiceInput.decode(
            PermissionStatusSnapshot.self,
            from: payload
        )

        XCTAssertEqual(snapshot.microphone, .denied)
        XCTAssertEqual(snapshot.accessibility, .authorized)
        XCTAssertEqual(snapshot.inputMonitoring, .notDetermined)
    }

    func testDefaultHotkeyDoesNotRequireInputMonitoring() {
        let requirements = PermissionRequirements(hotkey: "control+shift+space")

        XCTAssertEqual(requirements.required, [.microphone, .accessibility])
    }

    func testFunctionHotkeyUsesAccessibilityForEventListening() {
        let requirements = PermissionRequirements(hotkey: "fn")

        XCTAssertEqual(requirements.required, [.microphone, .accessibility])
    }

    func testUnresolvedPermissionCanRequestAgain() {
        XCTAssertTrue(SystemPermissionState.notDetermined.canRequest)
        XCTAssertTrue(SystemPermissionState.denied.canRequest)
        XCTAssertFalse(SystemPermissionState.restricted.canRequest)
        XCTAssertFalse(SystemPermissionState.authorized.canRequest)
    }

    func testReadyIgnoresOptionalInputMonitoring() {
        let snapshot = PermissionStatusSnapshot(
            schemaVersion: 1,
            subjectExecutable: "/tmp/voice-input",
            microphone: .authorized,
            accessibility: .authorized,
            inputMonitoring: .denied
        )

        XCTAssertTrue(
            snapshot.isReady(for: PermissionRequirements(hotkey: "control+shift+space"))
        )
        XCTAssertTrue(snapshot.isReady(for: PermissionRequirements(hotkey: "fn")))
    }
}
