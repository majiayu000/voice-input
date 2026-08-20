import SwiftUI

struct StatusMark: View {
    var recording = false
    var muted = false

    var body: some View {
        Text("听")
            .font(.custom("Songti SC", size: 12).weight(.semibold))
            .foregroundStyle(foreground)
            .frame(width: 22, height: 16)
            .background(background)
            .clipShape(RoundedRectangle(cornerRadius: 2, style: .continuous))
            .accessibilityHidden(true)
    }

    private var background: Color {
        if recording { return VoiceInputDesign.recording }
        if muted { return .secondary }
        return Color(nsColor: .labelColor)
    }

    private var foreground: Color {
        recording ? .white : Color(nsColor: .windowBackgroundColor)
    }
}

struct StatusHeader: View {
    let state: AppRuntimeState
    var detail: String? = nil

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            StatusMark(
                recording: state == .listening || state == .recognizing,
                muted: state == .paused
            )
            VStack(alignment: .leading, spacing: 2) {
                Text(state.title)
                    .font(.system(size: 13, weight: .semibold))
                Text(detail ?? state.detail)
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .accessibilityElement(children: .combine)
    }
}

struct InlineError: View {
    let message: String

    var body: some View {
        Label(message, systemImage: "exclamationmark.triangle.fill")
            .font(.system(size: 12))
            .foregroundStyle(.red)
            .fixedSize(horizontal: false, vertical: true)
            .accessibilityLabel("出现问题：\(message)")
    }
}

struct InlineSuccess: View {
    let message: String

    var body: some View {
        Label(message, systemImage: "checkmark.circle.fill")
            .font(.system(size: 12, weight: .medium))
            .foregroundStyle(VoiceInputDesign.success)
            .accessibilityLabel("完成：\(message)")
    }
}

struct TransientNotice: View {
    let message: String

    var body: some View {
        Label(message, systemImage: "checkmark.circle.fill")
            .font(.system(size: 12, weight: .medium))
            .foregroundStyle(.primary)
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .background(.regularMaterial)
            .overlay(
                RoundedRectangle(cornerRadius: VoiceInputDesign.cornerSmall)
                    .stroke(Color(nsColor: .separatorColor), lineWidth: 1)
            )
            .clipShape(RoundedRectangle(cornerRadius: VoiceInputDesign.cornerSmall))
            .shadow(color: Color(nsColor: .shadowColor).opacity(0.12), radius: 10, y: 4)
            .accessibilityAddTraits(.isStaticText)
    }
}

struct LoadingRows: View {
    var rows = 4

    var body: some View {
        VStack(spacing: 0) {
            ForEach(0..<rows, id: \.self) { index in
                HStack(spacing: 12) {
                    RoundedRectangle(cornerRadius: 3)
                        .fill(Color(nsColor: .quaternaryLabelColor))
                        .frame(width: 18, height: 18)
                    VStack(alignment: .leading, spacing: 5) {
                        RoundedRectangle(cornerRadius: 2)
                            .fill(Color(nsColor: .quaternaryLabelColor))
                            .frame(width: index.isMultiple(of: 2) ? 112 : 152, height: 9)
                        RoundedRectangle(cornerRadius: 2)
                            .fill(Color(nsColor: .quaternaryLabelColor).opacity(0.65))
                            .frame(width: 210, height: 7)
                    }
                    Spacer()
                }
                .padding(.vertical, 10)
                if index < rows - 1 { Divider() }
            }
        }
        .accessibilityElement()
        .accessibilityLabel("正在读取本地状态")
    }
}

struct PermissionRow: View {
    let kind: PermissionKind
    let state: SystemPermissionState
    let required: Bool
    let request: () -> Void
    let openSettings: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: kind.symbol)
                .font(.system(size: 16))
                .frame(width: 24)
                .foregroundStyle(state.isAuthorized ? .green : .secondary)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text(kind.title).font(.system(size: 13, weight: .medium))
                Text(required ? kind.explanation : "\(kind.explanation) 当前快捷键不需要此权限。")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 12)
            if state.isAuthorized {
                Label("已允许", systemImage: "checkmark.circle.fill")
                    .font(.system(size: 12))
                    .foregroundStyle(.green)
            } else {
                Text(state.statusText)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                if state.canRequest || state == .unknown {
                    Button(state == .unknown ? "重试" : "允许", action: request)
                }
                Button("打开设置", action: openSettings)
            }
        }
        .padding(.vertical, 8)
    }
}

struct ModelRow: View {
    let model: ModelSnapshot
    let busy: Bool
    var progress: Double? = nil
    let install: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: model.active ? "checkmark.circle.fill" : "circle")
                .foregroundStyle(model.active ? .green : .secondary)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(modelName(model.preset))
                        .font(.system(size: 13, weight: .medium))
                    if model.preset == "balanced" {
                        Text("推荐")
                            .font(.system(size: 10, weight: .medium))
                            .foregroundStyle(.secondary)
                    }
                }
                Text("\(model.sizeLabel) · \(modelDescription(model.preset))")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
            }
            Spacer()
            if let progress {
                VStack(alignment: .trailing, spacing: 3) {
                    ProgressView(value: progress)
                        .frame(width: 82)
                    Text("\(Int(progress * 100))%")
                        .font(.system(size: 10, design: .monospaced))
                        .foregroundStyle(.secondary)
                }
                .accessibilityLabel("\(modelName(model.preset))下载进度")
                .accessibilityValue("百分之\(Int(progress * 100))")
            } else if model.active {
                Text("正在使用").font(.system(size: 12)).foregroundStyle(.secondary)
            } else {
                Button(model.installed ? "使用" : "下载", action: install)
                    .disabled(busy)
            }
        }
        .padding(.vertical, 8)
        .accessibilityElement(children: .contain)
    }
}

func modelName(_ preset: String) -> String {
    switch preset {
    case "fast": "快速"
    case "quality": "高质量"
    default: "均衡"
    }
}

func modelDescription(_ preset: String) -> String {
    switch preset {
    case "fast": "速度最快，适合先试用"
    case "quality": "准确度更高，需要更多内存"
    default: "适合日常中英文输入"
    }
}
