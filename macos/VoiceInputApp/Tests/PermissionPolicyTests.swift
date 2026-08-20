import XCTest
@testable import VoiceInputApp

final class PermissionPolicyTests: XCTestCase {
    func testDefaultHotkeyDoesNotRequireInputMonitoring() {
        let requirements = PermissionRequirements(hotkey: "control+shift+space")

        XCTAssertEqual(requirements.required, [.microphone, .accessibility])
    }

    func testFunctionHotkeyRequiresInputMonitoring() {
        let requirements = PermissionRequirements(hotkey: "fn")

        XCTAssertEqual(
            requirements.required,
            [.microphone, .accessibility, .inputMonitoring]
        )
    }

    func testOnlyNotDeterminedPermissionCanRequestAgain() {
        XCTAssertTrue(SystemPermissionState.notDetermined.canRequest)
        XCTAssertFalse(SystemPermissionState.denied.canRequest)
        XCTAssertFalse(SystemPermissionState.restricted.canRequest)
        XCTAssertFalse(SystemPermissionState.authorized.canRequest)
    }

    func testReadyIgnoresOptionalInputMonitoring() {
        let snapshot = PermissionStatusSnapshot(
            microphone: .authorized,
            accessibility: .authorized,
            inputMonitoring: .denied
        )

        XCTAssertTrue(
            snapshot.isReady(for: PermissionRequirements(hotkey: "control+shift+space"))
        )
        XCTAssertFalse(snapshot.isReady(for: PermissionRequirements(hotkey: "fn")))
    }
}
