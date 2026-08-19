import Foundation

struct ControlSnapshot: Decodable, Equatable {
    let schemaVersion: Int
    let generatedAtMs: UInt64
    let service: ServiceSnapshot
    let settings: ControlSettings
    let models: [ModelSnapshot]
    let recentRuntime: RuntimeSnapshot?
}

struct ServiceSnapshot: Decodable, Equatable {
    let installed: Bool
    let loaded: Bool
    let launchdState: String?
    let pid: UInt32?
    let lastExitCode: Int?
    let runtime: RuntimeSnapshot?
    let paths: ServicePaths
}

struct ServicePaths: Decodable, Equatable {
    let dataDir: String
    let config: String
    let stdoutLog: String
    let stderrLog: String
    let runtimeStatus: String
    let latencyHistory: String
}

struct ControlSettings: Decodable, Equatable {
    let configPath: String
    let hotkey: String
    let audibleFeedback: Bool
    let language: String
    let insertionMode: String
    let activeModelPath: String?
    let refinerEnabled: Bool
    let refinerBaseUrl: String
    let refinerModel: String
    let refinerApiKeyEnv: String?
    let refinerAllowRemote: Bool
    let refinerFailureMode: String
    let refinerSystemPrompt: String
    let vadEnabled: Bool
}

struct ModelSnapshot: Decodable, Equatable, Identifiable {
    var id: String { preset }
    let preset: String
    let fileName: String
    let sizeBytes: UInt64
    let description: String
    let path: String
    let installed: Bool
    let active: Bool

    var sizeLabel: String {
        ByteCountFormatter.string(fromByteCount: Int64(sizeBytes), countStyle: .file)
    }
}

struct RuntimeSnapshot: Decodable, Equatable {
    let schemaVersion: Int
    let pid: UInt32
    let phase: String
    let startedAtMs: UInt64
    let updatedAtMs: UInt64
    let hotkey: String
    let activeSession: String?
    let sessionsCompleted: UInt64
    let lastLatency: LatencyReport?
    let lastText: String?
    let lastError: String?
}

struct LatencyReport: Decodable, Equatable {
    let captureMs: UInt64
    let firstAudioMs: UInt64?
    let finalizeMs: UInt64
    let refineMs: UInt64
    let insertMs: UInt64
    let stopToInsertedMs: UInt64
    let totalMs: UInt64
    let audioChunks: UInt64
    let audioSamples: UInt64
    let droppedChunks: UInt64
}

struct SettingsPatch: Encodable {
    var hotkey: String?
    var audibleFeedback: Bool?
    var language: String?
    var insertionMode: String?
    var refinerEnabled: Bool?
    var refinerBaseUrl: String?
    var refinerModel: String?
    var refinerApiKeyEnv: String?
    var refinerAllowRemote: Bool?
    var refinerFailureMode: String?
    var refinerSystemPrompt: String?
    var vadEnabled: Bool?
}

enum AppRuntimeState: Equatable {
    case needsSetup(String)
    case paused
    case starting
    case ready
    case listening
    case recognizing
    case error(String)

    var title: String {
        switch self {
        case .needsSetup: "还需要一步"
        case .paused: "已暂停"
        case .starting: "正在准备本地模型"
        case .ready: "本机输入法 · 就绪"
        case .listening: "正在听"
        case .recognizing: "正在本机识别"
        case .error: "需要处理"
        }
    }

    var detail: String {
        switch self {
        case .needsSetup(let reason): reason
        case .paused: "恢复后即可按住 Fn 说话"
        case .starting: "第一次加载可能需要十几秒"
        case .ready: "按住 Fn 说话，松开后写入"
        case .listening: "松开 Fn 后写入当前光标"
        case .recognizing: "文字仍在这台 Mac 上处理"
        case .error(let reason): reason
        }
    }
}

extension JSONDecoder {
    static var voiceInput: JSONDecoder {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return decoder
    }
}

extension JSONEncoder {
    static var voiceInput: JSONEncoder {
        let encoder = JSONEncoder()
        encoder.keyEncodingStrategy = .convertToSnakeCase
        return encoder
    }
}
