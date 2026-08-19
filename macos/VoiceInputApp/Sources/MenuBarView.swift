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

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            StatusHeader(state: model.runtimeState)
                .padding(.horizontal, 14)
                .padding(.vertical, 12)

            Divider()

            if let runtime = model.activeRuntime,
               let text = runtime.lastText,
               !text.isEmpty {
                VStack(alignment: .leading, spacing: 3) {
                    Text("最近一次")
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
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

            VStack(spacing: 2) {
                Button(action: model.toggleService) {
                    Label(
                        model.snapshot?.service.loaded == true ? "暂停语音输入" : "恢复语音输入",
                        systemImage: model.snapshot?.service.loaded == true ? "pause.circle" : "play.circle"
                    )
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .buttonStyle(.plain)
                .padding(.horizontal, 14)
                .padding(.vertical, 7)
                .disabled(model.isBusy)

                SettingsLink {
                    Label("设置…", systemImage: "gearshape")
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .buttonStyle(.plain)
                .padding(.horizontal, 14)
                .padding(.vertical, 7)

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
    }
}
