import AppKit
import SwiftUI

struct MenuBarLabel: View {
    @ObservedObject var model: AppModel

    var body: some View {
        StatusMark(
            recording: model.runtimeState == .listening || model.runtimeState == .recognizing,
            muted: model.runtimeState == .paused
        )
        .accessibilityLabel("Voice Input，\(model.runtimeState.title)")
    }
}

struct MenuBarPanel: View {
    @ObservedObject var model: AppModel
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            StatusHeader(state: model.runtimeState)
                .padding(.horizontal, 14)
                .padding(.vertical, 12)

            Divider()

            if model.isBusy, let label = model.busyLabel {
                VStack(alignment: .leading, spacing: 6) {
                    HStack(spacing: 8) {
                        ProgressView().controlSize(.small)
                        Text(label).font(.system(size: 12, weight: .medium))
                    }
                    if let progress = model.modelProgress {
                        ProgressView(value: progress)
                            .accessibilityLabel(label)
                            .accessibilityValue("百分之\(Int(progress * 100))")
                    }
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 10)
                Divider()
            }

            if let runtime = model.activeRuntime,
               let text = runtime.lastText,
               !text.isEmpty {
                VStack(alignment: .leading, spacing: 5) {
                    HStack {
                        Text("最近一次")
                            .font(.system(size: 11))
                            .foregroundStyle(.secondary)
                        Spacer()
                        Button("复制") { model.copyRecentText() }
                            .buttonStyle(.plain)
                            .font(.system(size: 11, weight: .medium))
                            .foregroundStyle(.secondary)
                            .keyboardShortcut("c", modifiers: [.command, .shift])
                    }
                    Text(text)
                        .font(.system(size: 13))
                        .lineLimit(3)
                    if let latency = runtime.lastLatency {
                        Text("松开到写入 \(latency.stopToInsertedMs) 毫秒")
                            .font(.system(size: 10, design: .monospaced))
                            .foregroundStyle(.secondary)
                    }
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 10)
                Divider()
            }

            if let error = model.errorMessage {
                InlineError(message: error)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 10)
                Divider()
            }

            if let toast = model.toastMessage {
                InlineSuccess(message: toast)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 9)
                    .transition(.opacity)
                Divider()
            }

            if showsPrimaryAction {
                Button(action: model.performPrimaryAction) {
                    Label(primaryActionTitle, systemImage: primaryActionSymbol)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.regular)
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
                .disabled(model.isBusy)
                Divider()
            }

            VStack(spacing: 2) {
                if showsPauseAction {
                    Button(action: model.toggleService) {
                        Label("暂停语音输入", systemImage: "pause.circle")
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .buttonStyle(.plain)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 7)
                    .disabled(model.isBusy)
                }

                SettingsLink {
                    Label("设置…", systemImage: "gearshape")
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .buttonStyle(.plain)
                .padding(.horizontal, 14)
                .padding(.vertical, 7)
                .keyboardShortcut(",", modifiers: .command)

                Button(action: model.showOnboarding) {
                    Label("重新查看使用引导", systemImage: "questionmark.circle")
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .buttonStyle(.plain)
                .padding(.horizontal, 14)
                .padding(.vertical, 7)
            }
            .font(.system(size: 13))

            Divider()

            HStack {
                Text(model.snapshot.map { "\($0.settings.hotkey.uppercased()) · \($0.settings.language) · \($0.settings.refinerEnabled ? "LLM 已开启" : "不发送 LLM")" } ?? "本地优先")
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundStyle(.secondary)
                Spacer()
                Button("退出") { model.quit() }
                    .buttonStyle(.plain)
                    .font(.system(size: 12))
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 9)
        }
        .frame(width: 300)
        .background(Color(nsColor: .windowBackgroundColor))
        .animation(reduceMotion ? nil : VoiceInputDesign.stateAnimation, value: model.toastMessage)
        .animation(reduceMotion ? nil : VoiceInputDesign.stateAnimation, value: model.runtimeState)
    }

    private var showsPrimaryAction: Bool {
        switch model.runtimeState {
        case .needsSetup, .paused, .error: true
        default: false
        }
    }

    private var showsPauseAction: Bool {
        switch model.runtimeState {
        case .ready, .listening, .recognizing: true
        default: false
        }
    }

    private var primaryActionTitle: String {
        switch model.runtimeState {
        case .needsSetup: "继续设置"
        case .paused: "恢复语音输入"
        case .error: "重试"
        default: ""
        }
    }

    private var primaryActionSymbol: String {
        switch model.runtimeState {
        case .needsSetup: "arrow.right.circle"
        case .paused: "play.circle"
        case .error: "arrow.clockwise.circle"
        default: "circle"
        }
    }
}
