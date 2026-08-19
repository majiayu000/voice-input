import AppKit
import SwiftUI

@MainActor
final class OnboardingWindowController {
    static let shared = OnboardingWindowController()
    private var window: NSWindow?

    func show(model: AppModel) {
        if let window {
            window.makeKeyAndOrderFront(nil)
            NSApp.activate(ignoringOtherApps: true)
            return
        }
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 620, height: 470),
            styleMask: [.titled, .closable, .miniaturizable],
            backing: .buffered,
            defer: false
        )
        window.title = "开始使用 Voice Input"
        window.center()
        window.isReleasedWhenClosed = false
        window.contentView = NSHostingView(rootView: OnboardingView(model: model))
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        self.window = window
    }

    func close() {
        window?.close()
        window = nil
    }
}

private enum OnboardingStep: Int, CaseIterable {
    case privacy
    case permissions
    case model
    case trial
    case launch

    var title: String {
        switch self {
        case .privacy: "隐私"
        case .permissions: "权限"
        case .model: "模型"
        case .trial: "试说"
        case .launch: "启动"
        }
    }
}

struct OnboardingView: View {
    @ObservedObject var model: AppModel
    @State private var step: OnboardingStep = .privacy
    @State private var trialText = "把光标放在这里，然后按住 Fn 说话。"
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                ForEach(OnboardingStep.allCases, id: \.rawValue) { item in
                    HStack(spacing: 5) {
                        Image(systemName: item.rawValue < step.rawValue ? "checkmark.circle.fill" : "circle.fill")
                        Text(item.title)
                    }
                    .font(.system(size: 11, weight: item == step ? .semibold : .regular))
                    .foregroundStyle(item.rawValue <= step.rawValue ? .primary : .tertiary)
                    if item != .launch { Divider().frame(width: 24) }
                }
            }
            .padding(.horizontal, 28)
            .padding(.vertical, 16)

            Divider()

            Group {
                switch step {
                case .privacy: privacyStep
                case .permissions: permissionStep
                case .model: modelStep
                case .trial: trialStep
                case .launch: launchStep
                }
            }
            .id(step)
            .transition(.opacity)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .padding(28)

            Divider()

            HStack {
                if step != .privacy {
                    Button("上一步") { move(-1) }
                }
                Spacer()
                if let error = model.errorMessage {
                    Text(error)
                        .font(.system(size: 11))
                        .foregroundStyle(.red)
                        .lineLimit(2)
                        .frame(maxWidth: 280, alignment: .trailing)
                }
                if step == .launch {
                    Button("开始使用") { model.finishOnboarding() }
                        .buttonStyle(.borderedProminent)
                        .disabled(!canContinue)
                } else {
                    Button("继续") { move(1) }
                        .buttonStyle(.borderedProminent)
                        .disabled(!canContinue)
                }
            }
            .padding(.horizontal, 24)
            .padding(.vertical, 14)
        }
        .frame(minWidth: 620, minHeight: 470)
        .task { await model.refresh() }
        .animation(reduceMotion ? nil : VoiceInputDesign.stateAnimation, value: step)
    }

    private var privacyStep: some View {
        VStack(alignment: .leading, spacing: 20) {
            Text("你的声音留在这台 Mac 上")
                .font(.system(size: 24, weight: .semibold))
            Text("Voice Input 默认在本机完成语音识别，不保存录音，也不会把文字发送给 LLM。以后只有在你主动配置并开启文本润色时，文字才会发送到指定服务。")
                .font(.system(size: 14))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: 500, alignment: .leading)
            VStack(alignment: .leading, spacing: 10) {
                Label("默认不保存原始音频", systemImage: "waveform.badge.minus")
                Label("本地 Whisper 模型完成识别", systemImage: "desktopcomputer")
                Label("远程能力始终需要你明确开启", systemImage: "lock")
            }
            .font(.system(size: 13))
        }
    }

    private var permissionStep: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("允许三项系统权限")
                .font(.system(size: 22, weight: .semibold))
            Text("每项权限只用于下方写明的功能。授权后回到这里，状态会自动更新。")
                .font(.system(size: 13))
                .foregroundStyle(.secondary)
                .padding(.bottom, 8)
            ForEach(PermissionKind.allCases) { kind in
                PermissionRow(
                    kind: kind,
                    granted: granted(kind),
                    request: { model.request(kind) },
                    openSettings: { model.openPermissionSettings(kind) }
                )
                if kind != .inputMonitoring { Divider() }
            }
            Button("重新检查权限") { Task { await model.refresh() } }
                .padding(.top, 6)
        }
    }

    private var modelStep: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("选择本地识别模型")
                .font(.system(size: 22, weight: .semibold))
            Text("模型只下载一次，并在本机运行。均衡模型适合大多数用户。")
                .font(.system(size: 13))
                .foregroundStyle(.secondary)
                .padding(.bottom, 8)
            ForEach(model.snapshot?.models ?? []) { item in
                ModelRow(
                    model: item,
                    busy: model.isBusy,
                    progress: model.downloadingPreset == item.preset ? model.modelProgress : nil
                ) {
                    model.installModel(item.preset)
                }
                if item.id != model.snapshot?.models.last?.id { Divider() }
            }
            if model.isBusy, let label = model.busyLabel {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text(label).font(.system(size: 12)).foregroundStyle(.secondary)
                }
                .padding(.top, 8)
            }
        }
    }

    private var trialStep: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("试着说一句")
                .font(.system(size: 22, weight: .semibold))
            Text("先把光标放进下面的文本框，然后按住 Fn 说话。松开后，文字应当出现在光标位置。")
                .font(.system(size: 13))
                .foregroundStyle(.secondary)
            TextEditor(text: $trialText)
                .font(.system(size: 14))
                .frame(height: 110)
                .overlay(RoundedRectangle(cornerRadius: 5).stroke(Color(nsColor: .separatorColor)))
            StatusHeader(state: model.runtimeState)
            if let text = model.activeRuntime?.lastText {
                Label("刚刚写入：\(text)", systemImage: "checkmark.circle.fill")
                    .font(.system(size: 12))
                    .foregroundStyle(.green)
            }
            if model.snapshot?.service.loaded != true {
                Button("启动本地输入") { Task { await model.ensureInstalledAndStarted() } }
                    .buttonStyle(.borderedProminent)
            }
        }
    }

    private var launchStep: some View {
        VStack(alignment: .leading, spacing: 20) {
            Text("已经可以使用")
                .font(.system(size: 24, weight: .semibold))
            Text("Voice Input 平时只留在菜单栏。看到“听”就表示它在那里，按住 Fn 即可开始。")
                .font(.system(size: 14))
                .foregroundStyle(.secondary)
                .frame(maxWidth: 500, alignment: .leading)
            Toggle("登录 Mac 时自动启动", isOn: Binding(
                get: { model.launchAtLogin },
                set: { model.setLaunchAtLogin($0) }
            ))
            .toggleStyle(.switch)
            Label("默认不占用 Dock，也不会保存语音历史。", systemImage: "menubar.rectangle")
                .font(.system(size: 12))
                .foregroundStyle(.secondary)
        }
    }

    private var canContinue: Bool {
        switch step {
        case .permissions: model.permissions.ready
        case .model: model.snapshot?.models.contains(where: \.active) == true && !model.isBusy
        case .trial: model.runtimeState == .ready || model.activeRuntime?.sessionsCompleted ?? 0 > 0
        default: true
        }
    }

    private func granted(_ kind: PermissionKind) -> Bool {
        switch kind {
        case .microphone: model.permissions.microphone
        case .accessibility: model.permissions.accessibility
        case .inputMonitoring: model.permissions.inputMonitoring
        }
    }

    private func move(_ offset: Int) {
        guard let next = OnboardingStep(rawValue: step.rawValue + offset) else { return }
        if step == .privacy && offset > 0 {
            model.apply(SettingsPatch(hotkey: "fn"), restart: false)
        }
        if next == .trial {
            Task { await model.ensureInstalledAndStarted() }
        }
        step = next
    }
}
