import AppKit
import ServiceManagement
import SwiftUI

@MainActor
final class AppModel: ObservableObject {
    static let shared = AppModel()

    @Published private(set) var snapshot: ControlSnapshot?
    @Published private(set) var permissions = PermissionSnapshot.unknown
    @Published private(set) var isBusy = false
    @Published private(set) var busyLabel: String?
    @Published var errorMessage: String?
    @Published var llmProbe: LLMProbe?
    @Published var launchAtLogin = false
    @Published private(set) var downloadingPreset: String?
    @Published private(set) var modelProgress: Double?

    private let bridge = RuntimeBridge.shared
    private let permissionService = PermissionService()
    private var pollTask: Task<Void, Never>?
    private var started = false
    private var lastSessionsCompleted: UInt64 = 0

    var runtimeState: AppRuntimeState {
        guard let snapshot else { return .starting }
        if !snapshot.models.contains(where: \.active) {
            return .needsSetup("请选择并准备一个本地识别模型")
        }
        if !permissions.microphone {
            return .needsSetup("需要麦克风权限才能接收语音")
        }
        if !permissions.accessibility {
            return .needsSetup("需要辅助功能权限才能写入文字")
        }
        if !permissions.inputMonitoring {
            return .needsSetup("需要输入监控权限才能监听 Fn")
        }
        guard snapshot.service.installed else {
            return .needsSetup("本地运行组件尚未安装")
        }
        guard snapshot.service.loaded else { return .paused }
        guard let runtime = snapshot.service.runtime ?? snapshot.recentRuntime else {
            if let code = snapshot.service.lastExitCode, code != 0 {
                return .error("本地运行组件退出，代码 \(code)。请打开诊断查看原因。")
            }
            return .starting
        }
        if let error = runtime.lastError, !error.isEmpty {
            return .error(friendlyError(error))
        }
        switch runtime.phase {
        case "capturing": return .listening
        case "finalizing": return .recognizing
        case "ready": return .ready
        case "starting": return .starting
        case "stopping", "stopped": return .paused
        default: return .starting
        }
    }

    var activeRuntime: RuntimeSnapshot? {
        snapshot?.service.runtime ?? snapshot?.recentRuntime
    }

    var onboardingComplete: Bool {
        get { UserDefaults.standard.bool(forKey: "onboardingComplete") }
        set { UserDefaults.standard.set(newValue, forKey: "onboardingComplete") }
    }

    func start() {
        guard !started else { return }
        started = true
        refreshLoginItemState()
        Task {
            await refresh()
            if onboardingComplete {
                await ensureInstalledAndStarted()
            } else {
                OnboardingWindowController.shared.show(model: self)
            }
        }
        pollTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(1))
                await self?.refresh(silent: true)
            }
        }
    }

    func refresh(silent: Bool = false) async {
        permissions = permissionService.snapshot()
        do {
            let next = try await bridge.snapshot()
            let sessions = (next.service.runtime ?? next.recentRuntime)?.sessionsCompleted ?? 0
            let completedNow = sessions > lastSessionsCompleted
            lastSessionsCompleted = sessions
            snapshot = next
            CandidateOverlayController.shared.update(
                runtime: next.service.runtime ?? next.recentRuntime,
                completedNow: completedNow
            )
            if !silent { errorMessage = nil }
        } catch {
            if !silent { errorMessage = friendlyError(error.localizedDescription) }
        }
    }

    func ensureInstalledAndStarted() async {
        await perform("正在准备 Voice Input") {
            if self.snapshot?.service.installed != true {
                try await self.bridge.installService()
            }
            if self.snapshot?.service.loaded != true {
                try await self.bridge.startService()
            }
        }
    }

    func toggleService() {
        Task {
            if snapshot?.service.loaded == true {
                await perform("正在暂停") { try await self.bridge.stopService() }
            } else {
                await ensureInstalledAndStarted()
            }
        }
    }

    func apply(_ patch: SettingsPatch, restart: Bool = true) {
        Task {
            await perform("正在保存设置") {
                try await self.bridge.apply(patch)
                if restart, self.snapshot?.service.loaded == true {
                    try await self.bridge.startService()
                }
            }
        }
    }

    func installModel(_ preset: String) {
        Task {
            guard let model = snapshot?.models.first(where: { $0.preset == preset }) else { return }
            downloadingPreset = preset
            modelProgress = 0
            let progressTask = Task { [weak self] in
                await self?.pollModelProgress(model)
            }
            await perform("正在下载并校验 \(presetName(preset))") {
                try await self.bridge.installModel(preset)
                if self.snapshot?.service.installed != true {
                    try await self.bridge.installService()
                }
                try await self.bridge.startService()
            }
            progressTask.cancel()
            modelProgress = nil
            downloadingPreset = nil
        }
    }

    func request(_ permission: PermissionKind) {
        Task {
            switch permission {
            case .microphone:
                await permissionService.requestMicrophone()
            case .accessibility:
                permissionService.requestAccessibility()
            case .inputMonitoring:
                permissionService.requestInputMonitoring()
            }
            try? await Task.sleep(for: .milliseconds(500))
            permissions = permissionService.snapshot()
        }
    }

    func openPermissionSettings(_ permission: PermissionKind) {
        permissionService.openSettings(permission)
    }

    func setLaunchAtLogin(_ enabled: Bool) {
        do {
            if enabled {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
            refreshLoginItemState()
        } catch {
            launchAtLogin = SMAppService.mainApp.status == .enabled
            errorMessage = "无法更新登录启动：\(error.localizedDescription)"
        }
    }

    func testLLM() {
        Task {
            llmProbe = nil
            await perform("正在测试 LLM 连接") {
                self.llmProbe = try await self.bridge.testRefiner()
            }
        }
    }

    func saveAndTestLLM(_ patch: SettingsPatch) {
        Task {
            llmProbe = nil
            await perform("正在保存并测试 LLM") {
                try await self.bridge.apply(patch)
                self.llmProbe = try await self.bridge.testRefiner()
                if self.snapshot?.service.loaded == true {
                    try await self.bridge.startService()
                }
            }
        }
    }

    func copyDiagnostics() {
        Task {
            do {
                let raw = try await bridge.rawSnapshot()
                let permissionText = """

                permissions:
                  microphone: \(permissions.microphone)
                  accessibility: \(permissions.accessibility)
                  input_monitoring: \(permissions.inputMonitoring)
                """
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(raw + permissionText, forType: .string)
            } catch {
                errorMessage = friendlyError(error.localizedDescription)
            }
        }
    }

    func openLogs() {
        guard let path = snapshot?.service.paths.stderrLog else { return }
        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)])
    }

    func finishOnboarding() {
        onboardingComplete = true
        OnboardingWindowController.shared.close()
    }

    func showOnboarding() {
        OnboardingWindowController.shared.show(model: self)
    }

    func quit() {
        Task {
            if snapshot?.service.loaded == true {
                try? await bridge.stopService()
            }
            NSApplication.shared.terminate(nil)
        }
    }

    private func perform(
        _ label: String,
        operation: @escaping () async throws -> Void
    ) async {
        guard !isBusy else { return }
        isBusy = true
        busyLabel = label
        errorMessage = nil
        defer {
            isBusy = false
            busyLabel = nil
        }
        do {
            try await operation()
            await refresh()
        } catch {
            errorMessage = friendlyError(error.localizedDescription)
            await refresh(silent: true)
        }
    }

    private func refreshLoginItemState() {
        launchAtLogin = SMAppService.mainApp.status == .enabled
    }

    private func pollModelProgress(_ model: ModelSnapshot) async {
        let destination = URL(fileURLWithPath: model.path)
        let partial = destination
            .deletingLastPathComponent()
            .appendingPathComponent(".\(destination.lastPathComponent).partial")
        while !Task.isCancelled {
            let bytes = (try? FileManager.default.attributesOfItem(atPath: partial.path)[.size] as? NSNumber)?
                .doubleValue ?? 0
            modelProgress = min(max(bytes / Double(model.sizeBytes), 0), 1)
            try? await Task.sleep(for: .milliseconds(250))
        }
    }
}

private func presetName(_ preset: String) -> String {
    switch preset {
    case "fast": "快速模型"
    case "quality": "高质量模型"
    default: "均衡模型"
    }
}

private func friendlyError(_ message: String) -> String {
    let lower = message.lowercased()
    if lower.contains("accessibility") || lower.contains("event tap") {
        return "辅助功能或输入监控尚未允许。授权后请重新启动 Voice Input。"
    }
    if lower.contains("microphone") || lower.contains("audio") {
        return "无法使用麦克风。请检查麦克风权限和当前输入设备。"
    }
    if lower.contains("model") && lower.contains("does not exist") {
        return "本地模型不可用。请在“识别”中重新下载模型。"
    }
    if lower.contains("launchctl") {
        return "本地运行组件没有启动。请重试；如果仍失败，请打开诊断。"
    }
    return message
}
