import AppKit
import ServiceManagement
import SwiftUI

@MainActor
final class AppModel: ObservableObject {
    static let shared = AppModel()

    @Published private(set) var snapshot: ControlSnapshot?
    @Published private(set) var liveRuntime: RuntimeSnapshot?
    @Published private(set) var permissions = PermissionStatusSnapshot.unknown
    @Published private(set) var statusReadError: String?
    @Published private(set) var isBusy = false
    @Published private(set) var busyLabel: String?
    @Published var errorMessage: String?
    @Published var llmProbe: LLMProbe?
    @Published var launchAtLogin = false
    @Published private(set) var toastMessage: String?
    @Published private(set) var downloadingPreset: String?
    @Published private(set) var modelProgress: Double?

    private let bridge = RuntimeBridge.shared
    private let permissionSettings = PermissionSettingsService()
    private var pollTask: Task<Void, Never>?
    private var attemptedPermissions = Set<PermissionKind>()
    private var started = false
    private var lastSessionsCompleted: UInt64 = 0
    private var lastRuntimePID: UInt32?
    private var toastTask: Task<Void, Never>?
    private var refreshInProgress = false

    var permissionRequirements: PermissionRequirements {
        PermissionRequirements(hotkey: snapshot?.settings.hotkey ?? "control+shift+space")
    }

    var permissionsReady: Bool {
        permissions.isReady(for: permissionRequirements)
    }

    var hotkeyDisplayName: String {
        snapshot?.settings.hotkey == "fn" ? "Fn" : "Control + Shift + Space"
    }

    var runtimeDetail: String {
        switch runtimeState {
        case .paused: "恢复后即可按住 \(hotkeyDisplayName) 说话"
        case .ready: "按住 \(hotkeyDisplayName) 说话，松开后写入"
        case .listening: "松开 \(hotkeyDisplayName) 后写入当前光标"
        default: runtimeState.detail
        }
    }

    var runtimeState: AppRuntimeState {
        if let statusReadError {
            return .error(statusReadError)
        }
        guard let snapshot else { return .starting }
        if !snapshot.models.contains(where: \.active) {
            return .needsSetup("请选择并准备一个本地识别模型")
        }
        if permissions.microphone != .authorized {
            return .needsSetup("需要麦克风权限才能接收语音")
        }
        if permissions.accessibility != .authorized {
            return .needsSetup("需要辅助功能权限才能写入文字")
        }
        guard snapshot.service.installed else {
            return .needsSetup("本地运行组件尚未安装")
        }
        guard snapshot.service.loaded else { return .paused }
        guard let runtime = liveRuntime else {
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
        liveRuntime ?? snapshot?.recentRuntime
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
            var tick = 0
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(125))
                guard !Task.isCancelled, let self else { return }
                tick += 1
                if !self.isBusy {
                    await self.refreshLiveRuntime()
                    if tick.isMultiple(of: 40) {
                        await self.refresh(silent: true)
                    }
                }
            }
        }
    }

    func refresh(silent: Bool = false) async {
        if refreshInProgress {
            guard !silent else { return }
            while refreshInProgress, !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(25))
            }
            guard !Task.isCancelled else { return }
        }
        refreshInProgress = true
        defer { refreshInProgress = false }
        do {
            let next = try await bridge.snapshot()
            var permissionError: String?
            if next.service.installed {
                do {
                    permissions = try await readPermissions(helperPath: next.service.paths.binary)
                } catch {
                    permissions = .unknown
                    permissionError = RuntimeBridgeError.permissionStatusUnavailable(
                        error.localizedDescription
                    ).localizedDescription
                }
            } else {
                permissions = .unknown
            }
            snapshot = next
            updateLiveRuntime(next.service.runtime)
            statusReadError = permissionError
            if !silent {
                errorMessage = permissionError
            }
        } catch {
            permissions = .unknown
            let message = friendlyError(error.localizedDescription)
            statusReadError = message
            if !silent { errorMessage = message }
        }
    }

    private func refreshLiveRuntime() async {
        guard let snapshot, snapshot.service.loaded else {
            updateLiveRuntime(nil)
            return
        }
        do {
            let runtime = try await bridge.runtimeSnapshot(at: snapshot.service.paths.runtimeStatus)
            updateLiveRuntime(runtime?.pid == snapshot.service.pid ? runtime : nil)
        } catch {
            // The slower control snapshot reports persistent status-file failures.
        }
    }

    func ensureInstalledAndStarted() async {
        await perform("正在准备 Voice Input") {
            try await self.bridge.installService()
            try await self.bridge.startService()
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
            if errorMessage == nil { showToast("设置已保存") }
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
                try await self.bridge.installService()
                try await self.bridge.startService()
            }
            progressTask.cancel()
            modelProgress = nil
            downloadingPreset = nil
            if errorMessage == nil { showToast("\(presetName(preset))已准备好") }
        }
    }

    func request(_ permission: PermissionKind) {
        Task {
            await perform("正在请求\(permission.title)权限") {
                try await self.bridge.installService()
                let helperPath = try await self.bridge.snapshot().service.paths.binary
                self.attemptedPermissions.insert(permission)
                var next = try await self.bridge.requestPermission(
                    permission,
                    helperPath: helperPath
                )
                self.recordPermissionAttempt(permission, for: next)
                next.markDeniedIfUnchanged(permission)
                self.permissions = next
                if !next.state(for: permission).isAuthorized {
                    self.permissionSettings.openSettings(permission)
                    self.showToast("请在系统设置中允许 Voice Input Runtime")
                }
            }
        }
    }

    func openPermissionSettings(_ permission: PermissionKind) {
        permissionSettings.openSettings(permission)
    }

    func setLaunchAtLogin(_ enabled: Bool) {
        do {
            if enabled {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
            refreshLoginItemState()
            switch SMAppService.mainApp.status {
            case .enabled:
                showToast("已设置登录时启动")
            case .requiresApproval:
                errorMessage = "还需要在“系统设置 → 通用 → 登录项”中允许 Voice Input。"
            case .notRegistered, .notFound:
                if !enabled { showToast("已关闭登录时启动") }
            @unknown default:
                errorMessage = "macOS 没有返回登录项状态，请稍后重试。"
            }
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
                let raw = try await bridge.diagnosticSnapshot()
                let permissionText = """

                permissions:
                  subject: \(permissions.subjectExecutable)
                  microphone: \(permissions.microphone.rawValue)
                  accessibility: \(permissions.accessibility.rawValue)
                  input_monitoring: \(permissions.inputMonitoring.rawValue)
                """
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(raw + permissionText, forType: .string)
                showToast("诊断信息已复制")
            } catch {
                errorMessage = friendlyError(error.localizedDescription)
            }
        }
    }

    func openLogs() {
        guard let path = snapshot?.service.paths.stderrLog else { return }
        let log = URL(fileURLWithPath: path)
        if FileManager.default.fileExists(atPath: log.path) {
            NSWorkspace.shared.activateFileViewerSelecting([log])
        } else {
            NSWorkspace.shared.open(log.deletingLastPathComponent())
        }
    }

    func copyRecentText() {
        guard let text = activeRuntime?.lastText, !text.isEmpty else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
        showToast("最近文字已复制")
    }

    func performPrimaryAction() {
        switch runtimeState {
        case .needsSetup:
            showOnboarding()
        case .error:
            if permissionsReady {
                Task {
                    await perform("正在重新启动") {
                        if self.snapshot?.service.installed != true {
                            try await self.bridge.installService()
                        }
                        try await self.bridge.startService()
                    }
                }
            } else {
                showOnboarding()
            }
        case .paused:
            toggleService()
        case .starting, .ready, .listening, .recognizing:
            break
        }
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
            if snapshot?.service.loaded == true { try? await bridge.stopService() }
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

    private func readPermissions(helperPath: String) async throws -> PermissionStatusSnapshot {
        var permissionSnapshot = try await bridge.permissionSnapshot(helperPath: helperPath)
        for permission in PermissionKind.allCases
        where permissionWasAttempted(permission, for: permissionSnapshot) {
            permissionSnapshot.markDeniedIfUnchanged(permission)
        }
        return permissionSnapshot
    }

    private func updateLiveRuntime(_ runtime: RuntimeSnapshot?) {
        let completedNow: Bool
        if let runtime {
            completedNow = lastRuntimePID == runtime.pid
                && runtime.sessionsCompleted > lastSessionsCompleted
            lastRuntimePID = runtime.pid
            lastSessionsCompleted = runtime.sessionsCompleted
        } else {
            completedNow = false
            lastRuntimePID = nil
            lastSessionsCompleted = 0
        }
        liveRuntime = runtime
        CandidateOverlayController.shared.update(runtime: runtime, completedNow: completedNow)
        if completedNow { showToast("文字已写入") }
    }

    private func recordPermissionAttempt(
        _ permission: PermissionKind,
        for snapshot: PermissionStatusSnapshot
    ) {
        attemptedPermissions.insert(permission)
        guard let identity = permissionSubjectIdentity(snapshot) else { return }
        UserDefaults.standard.set(identity, forKey: "permissionAttempt.\(permission.rawValue)")
    }

    private func permissionWasAttempted(
        _ permission: PermissionKind,
        for snapshot: PermissionStatusSnapshot
    ) -> Bool {
        if attemptedPermissions.contains(permission) {
            return true
        }
        guard let identity = permissionSubjectIdentity(snapshot) else { return false }
        return UserDefaults.standard.string(
            forKey: "permissionAttempt.\(permission.rawValue)"
        ) == identity
    }

    private func permissionSubjectIdentity(
        _ snapshot: PermissionStatusSnapshot
    ) -> String? {
        guard !snapshot.subjectExecutable.isEmpty,
              let attributes = try? FileManager.default.attributesOfItem(
                  atPath: snapshot.subjectExecutable
              ),
              let size = attributes[.size] as? NSNumber,
              let modified = attributes[.modificationDate] as? Date else {
            return nil
        }
        return "launchd-v1|\(snapshot.subjectExecutable)|\(size)|\(modified.timeIntervalSince1970)"
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

    private func showToast(_ message: String) {
        toastTask?.cancel()
        toastMessage = message
        toastTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(1.5))
            guard !Task.isCancelled else { return }
            self?.toastMessage = nil
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
    if lower.contains("accessibility") || lower.contains("input monitoring") || lower.contains("event tap") {
        return "辅助功能尚未允许。授权 Voice Input Runtime 后请重新启动 Voice Input。"
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
    if lower.contains("timed out") || lower.contains("timeout") {
        return "操作等待超时。请检查网络或本地服务后重试。"
    }
    if lower.contains("sha-256") || lower.contains("checksum") {
        return "模型校验失败，未启用损坏文件。请重新下载。"
    }
    if lower.contains("connection") || lower.contains("network") || lower.contains("dns") {
        return "无法连接到服务。请检查地址、网络和服务是否正在运行。"
    }
    return message
}
