import AppKit

enum SystemPermissionState: String, Codable, Equatable {
    case notDetermined = "not_determined"
    case denied
    case restricted
    case authorized
    case unknown

    var canRequest: Bool { self == .notDetermined }
    var isAuthorized: Bool { self == .authorized }

    var statusText: String {
        switch self {
        case .notDetermined: "尚未请求"
        case .denied: "未允许"
        case .restricted: "受系统限制"
        case .authorized: "已允许"
        case .unknown: "无法读取"
        }
    }
}

struct PermissionStatusSnapshot: Codable, Equatable {
    let schemaVersion: Int
    let subjectExecutable: String
    var microphone: SystemPermissionState
    var accessibility: SystemPermissionState
    var inputMonitoring: SystemPermissionState

    static let unknown = PermissionStatusSnapshot(
        schemaVersion: 1,
        subjectExecutable: "",
        microphone: .unknown,
        accessibility: .unknown,
        inputMonitoring: .unknown
    )

    func state(for kind: PermissionKind) -> SystemPermissionState {
        switch kind {
        case .microphone: microphone
        case .accessibility: accessibility
        case .inputMonitoring: inputMonitoring
        }
    }

    func isReady(for requirements: PermissionRequirements) -> Bool {
        requirements.required.allSatisfy { state(for: $0).isAuthorized }
    }

    mutating func markDeniedIfUnchanged(_ kind: PermissionKind) {
        guard state(for: kind) == .notDetermined else { return }
        switch kind {
        case .microphone: microphone = .denied
        case .accessibility: accessibility = .denied
        case .inputMonitoring: inputMonitoring = .denied
        }
    }
}

struct PermissionRequirements: Equatable {
    let required: [PermissionKind]

    init(hotkey: String) {
        var required: [PermissionKind] = [.microphone, .accessibility]
        if hotkey == "fn" {
            required.append(.inputMonitoring)
        }
        self.required = required
    }
}

@MainActor
final class PermissionSettingsService {
    func openSettings(_ permission: PermissionKind) {
        let anchor: String
        switch permission {
        case .microphone: anchor = "Privacy_Microphone"
        case .accessibility: anchor = "Privacy_Accessibility"
        case .inputMonitoring: anchor = "Privacy_ListenEvent"
        }
        guard let url = URL(
            string: "x-apple.systempreferences:com.apple.preference.security?\(anchor)"
        ) else { return }
        NSWorkspace.shared.open(url)
    }
}

enum PermissionKind: String, CaseIterable, Identifiable {
    case microphone
    case accessibility
    case inputMonitoring

    var id: String { rawValue }

    var cliArgument: String {
        switch self {
        case .microphone: "microphone"
        case .accessibility: "accessibility"
        case .inputMonitoring: "input-monitoring"
        }
    }

    var title: String {
        switch self {
        case .microphone: "麦克风"
        case .accessibility: "辅助功能"
        case .inputMonitoring: "输入监控"
        }
    }

    var explanation: String {
        switch self {
        case .microphone: "接收你说的话；音频默认不保存。"
        case .accessibility: "把识别结果写入当前输入框。"
        case .inputMonitoring: "在其他应用中识别 Fn 的按下和松开。"
        }
    }

    var symbol: String {
        switch self {
        case .microphone: "mic"
        case .accessibility: "accessibility"
        case .inputMonitoring: "keyboard"
        }
    }
}
