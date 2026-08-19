import SwiftUI

private enum SettingsSection: String, CaseIterable, Identifiable {
    case general
    case recognition
    case text
    case diagnostics

    var id: String { rawValue }
    var title: String {
        switch self {
        case .general: "常规"
        case .recognition: "识别"
        case .text: "文本"
        case .diagnostics: "诊断"
        }
    }
    var symbol: String {
        switch self {
        case .general: "gearshape"
        case .recognition: "text.bubble"
        case .text: "textformat"
        case .diagnostics: "stethoscope"
        }
    }
}

struct SettingsRootView: View {
    @ObservedObject var model: AppModel
    @State private var selection: SettingsSection? = .general

    var body: some View {
        NavigationSplitView {
            List(SettingsSection.allCases, selection: $selection) { item in
                Label(item.title, systemImage: item.symbol).tag(item)
            }
            .navigationSplitViewColumnWidth(min: 150, ideal: 170, max: 210)
        } detail: {
            Group {
                switch selection ?? .general {
                case .general: GeneralSettings(model: model)
                case .recognition: RecognitionSettings(model: model)
                case .text: TextSettings(model: model)
                case .diagnostics: DiagnosticsSettings(model: model)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
        .frame(minWidth: 720, minHeight: 500)
        .task { await model.refresh() }
    }
}

private struct SettingsPage<Content: View>: View {
    let title: String
    @ViewBuilder let content: Content

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                Text(title).font(.system(size: 22, weight: .semibold))
                content
            }
            .padding(28)
            .frame(maxWidth: 620, alignment: .leading)
        }
    }
}

private struct GeneralSettings: View {
    @ObservedObject var model: AppModel

    var body: some View {
        SettingsPage(title: "常规") {
            StatusHeader(state: model.runtimeState)
            Form {
                Toggle("启用 Voice Input", isOn: Binding(
                    get: { model.snapshot?.service.loaded == true },
                    set: { _ in model.toggleService() }
                ))
                Picker("按住说话", selection: Binding(
                    get: { model.snapshot?.settings.hotkey ?? "fn" },
                    set: { model.apply(SettingsPatch(hotkey: $0)) }
                )) {
                    Text("Fn（功能键）").tag("fn")
                    Text("Control + Shift + Space").tag("control+shift+space")
                }
                Toggle("播放开始和完成提示音", isOn: Binding(
                    get: { model.snapshot?.settings.audibleFeedback ?? true },
                    set: { model.apply(SettingsPatch(audibleFeedback: $0)) }
                ))
                Toggle("登录 Mac 时自动启动", isOn: Binding(
                    get: { model.launchAtLogin },
                    set: { model.setLaunchAtLogin($0) }
                ))
                Picker("写入方式", selection: Binding(
                    get: { model.snapshot?.settings.insertionMode ?? "auto" },
                    set: { model.apply(SettingsPatch(insertionMode: $0)) }
                )) {
                    Text("自动（推荐）").tag("auto")
                    Text("仅辅助功能").tag("accessibility")
                    Text("仅剪贴板").tag("clipboard")
                }
            }
            .formStyle(.grouped)
            PermissionSummary(model: model)
        }
    }
}

private struct PermissionSummary: View {
    @ObservedObject var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("系统权限").font(.system(size: 13, weight: .semibold)).padding(.bottom, 4)
            ForEach(PermissionKind.allCases) { kind in
                PermissionRow(
                    kind: kind,
                    granted: granted(kind),
                    request: { model.request(kind) },
                    openSettings: { model.openPermissionSettings(kind) }
                )
                if kind != .inputMonitoring { Divider() }
            }
        }
    }

    private func granted(_ kind: PermissionKind) -> Bool {
        switch kind {
        case .microphone: model.permissions.microphone
        case .accessibility: model.permissions.accessibility
        case .inputMonitoring: model.permissions.inputMonitoring
        }
    }
}

private struct RecognitionSettings: View {
    @ObservedObject var model: AppModel

    var body: some View {
        SettingsPage(title: "语音识别") {
            Text("模型在本机运行。切换模型时会先下载并校验，成功后才会启用。")
                .font(.system(size: 13))
                .foregroundStyle(.secondary)
            VStack(spacing: 0) {
                ForEach(model.snapshot?.models ?? []) { item in
                    ModelRow(
                        model: item,
                        busy: model.isBusy,
                        progress: model.downloadingPreset == item.preset ? model.modelProgress : nil
                    ) { model.installModel(item.preset) }
                    if item.id != model.snapshot?.models.last?.id { Divider() }
                }
            }
            if model.isBusy, let label = model.busyLabel {
                HStack { ProgressView().controlSize(.small); Text(label) }
                    .font(.system(size: 12)).foregroundStyle(.secondary)
            }
            Form {
                Picker("识别语言", selection: Binding(
                    get: { model.snapshot?.settings.language ?? "auto" },
                    set: { model.apply(SettingsPatch(language: $0)) }
                )) {
                    Text("自动识别").tag("auto")
                    Text("简体中文").tag("zh")
                    Text("English").tag("en")
                }
                Toggle("自动忽略没有说话的片段", isOn: Binding(
                    get: { model.snapshot?.settings.vadEnabled ?? true },
                    set: { model.apply(SettingsPatch(vadEnabled: $0)) }
                ))
            }
            .formStyle(.grouped)
        }
    }
}

private struct TextSettings: View {
    @ObservedObject var model: AppModel
    @State private var enabled = false
    @State private var endpoint = "http://127.0.0.1:11434/v1"
    @State private var llmModel = ""
    @State private var keyEnvironment = ""
    @State private var allowRemote = false
    @State private var prompt = ""

    var body: some View {
        SettingsPage(title: "文本处理") {
            Toggle("使用 OpenAI 兼容服务润色文字", isOn: $enabled)
                .toggleStyle(.switch)
            Text(enabled ? "识别文字会发送到下方服务。使用本机地址时，文字不会离开这台 Mac。" : "当前使用原始识别结果，不产生 LLM 网络请求。")
                .font(.system(size: 12))
                .foregroundStyle(.secondary)
            Form {
                TextField("服务地址", text: $endpoint)
                    .disabled(!enabled)
                TextField("模型名称", text: $llmModel)
                    .disabled(!enabled)
                TextField("API Key 环境变量名称", text: $keyEnvironment)
                    .disabled(!enabled)
                Toggle("允许 HTTPS 远程地址", isOn: $allowRemote)
                    .disabled(!enabled)
                TextField("润色规则", text: $prompt, axis: .vertical)
                    .lineLimit(3...6)
                    .disabled(!enabled)
            }
            .formStyle(.grouped)
            if allowRemote && enabled {
                Label("远程服务会收到语音识别后的文字。请确认你信任这个服务。", systemImage: "exclamationmark.triangle")
                    .font(.system(size: 12))
                    .foregroundStyle(.orange)
            }
            HStack {
                Button("保存文本设置") { save() }
                    .buttonStyle(.borderedProminent)
                    .disabled(enabled && (endpoint.isEmpty || llmModel.isEmpty))
                Button("保存并测试连接") { saveAndTest() }
                    .disabled(!enabled || model.isBusy)
                if let probe = model.llmProbe {
                    Label("已连接 · \(probe.latencyMs) 毫秒", systemImage: "checkmark.circle.fill")
                        .font(.system(size: 12)).foregroundStyle(.green)
                }
            }
            if let error = model.errorMessage { InlineError(message: error) }
        }
        .onAppear(perform: load)
        .onChange(of: model.snapshot) { load() }
    }

    private func load() {
        guard let settings = model.snapshot?.settings else { return }
        enabled = settings.refinerEnabled
        endpoint = settings.refinerBaseUrl
        llmModel = settings.refinerModel
        keyEnvironment = settings.refinerApiKeyEnv ?? ""
        allowRemote = settings.refinerAllowRemote
        prompt = settings.refinerSystemPrompt
    }

    private func save() {
        model.apply(currentPatch())
    }

    private func saveAndTest() {
        model.saveAndTestLLM(currentPatch())
    }

    private func currentPatch() -> SettingsPatch {
        SettingsPatch(
            refinerEnabled: enabled,
            refinerBaseUrl: endpoint,
            refinerModel: llmModel,
            refinerApiKeyEnv: keyEnvironment,
            refinerAllowRemote: allowRemote,
            refinerSystemPrompt: prompt
        )
    }
}

private struct DiagnosticsSettings: View {
    @ObservedObject var model: AppModel

    var body: some View {
        SettingsPage(title: "诊断") {
            if let snapshot = model.snapshot {
                VStack(spacing: 0) {
                    DiagnosticRow("运行组件", value: snapshot.service.loaded ? "正在运行" : "已停止")
                    Divider()
                    DiagnosticRow("进程", value: snapshot.service.pid.map(String.init) ?? "—")
                    Divider()
                    DiagnosticRow("模型", value: snapshot.models.first(where: \.active)?.fileName ?? "未选择")
                    Divider()
                    DiagnosticRow("麦克风", value: model.permissions.microphone ? "已允许" : "未允许")
                    Divider()
                    DiagnosticRow("辅助功能", value: model.permissions.accessibility ? "已允许" : "未允许")
                    Divider()
                    DiagnosticRow("输入监控", value: model.permissions.inputMonitoring ? "已允许" : "未允许")
                    Divider()
                    DiagnosticRow("配置", value: snapshot.settings.configPath)
                    Divider()
                    DiagnosticRow("最近延迟", value: latency(snapshot))
                }
                .textSelection(.enabled)
                if let error = snapshot.service.runtime?.lastError { InlineError(message: error) }
                HStack {
                    Button("复制诊断信息") { model.copyDiagnostics() }
                    Button("在 Finder 中显示日志") { model.openLogs() }
                    Button("重新检查") { Task { await model.refresh() } }
                }
            } else {
                ProgressView("正在读取本地状态…")
            }
        }
    }

    private func latency(_ snapshot: ControlSnapshot) -> String {
        guard let value = (snapshot.service.runtime ?? snapshot.recentRuntime)?.lastLatency else { return "暂无" }
        return "松开到写入 \(value.stopToInsertedMs) 毫秒 · 总计 \(value.totalMs) 毫秒"
    }
}

private struct DiagnosticRow: View {
    let label: String
    let value: String

    init(_ label: String, value: String) {
        self.label = label
        self.value = value
    }

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 16) {
            Text(label).frame(width: 90, alignment: .leading)
            Text(value).foregroundStyle(.secondary).frame(maxWidth: .infinity, alignment: .leading)
        }
        .font(.system(size: 12))
        .padding(.vertical, 9)
    }
}
