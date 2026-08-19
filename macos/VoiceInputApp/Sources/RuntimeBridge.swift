import Foundation

actor RuntimeBridge {
    static let shared = RuntimeBridge()

    private let helperURL: URL

    init(helperURL: URL? = nil) {
        self.helperURL = helperURL ?? Self.discoverHelper()
    }

    func snapshot() async throws -> ControlSnapshot {
        let data = try await run(["control", "snapshot"])
        return try JSONDecoder.voiceInput.decode(ControlSnapshot.self, from: data)
    }

    func installService() async throws {
        _ = try await run(["install"])
    }

    func startService() async throws {
        _ = try await run(["start"])
    }

    func stopService() async throws {
        _ = try await run(["stop"])
    }

    func installModel(_ preset: String) async throws {
        _ = try await run(["model", "install", "--preset", preset])
    }

    func apply(_ patch: SettingsPatch) async throws {
        let input = try JSONEncoder.voiceInput.encode(patch)
        _ = try await run(["control", "apply"], input: input)
    }

    func testRefiner() async throws -> LLMProbe {
        let data = try await run(["control", "test-refiner"])
        return try JSONDecoder.voiceInput.decode(LLMProbe.self, from: data)
    }

    func rawSnapshot() async throws -> String {
        let data = try await run(["control", "snapshot"])
        return String(decoding: data, as: UTF8.self)
    }

    private func run(_ arguments: [String], input: Data? = nil) async throws -> Data {
        let executable = helperURL
        return try await Task.detached(priority: .userInitiated) {
            guard FileManager.default.isExecutableFile(atPath: executable.path) else {
                throw RuntimeBridgeError.helperMissing(executable.path)
            }

            let process = Process()
            let stdout = Pipe()
            let stderr = Pipe()
            process.executableURL = executable
            process.arguments = arguments
            process.standardOutput = stdout
            process.standardError = stderr

            let stdin = input.map { _ in Pipe() }
            process.standardInput = stdin
            try process.run()
            if let input, let stdin {
                stdin.fileHandleForWriting.write(input)
                try? stdin.fileHandleForWriting.close()
            }
            process.waitUntilExit()
            let output = stdout.fileHandleForReading.readDataToEndOfFile()
            let failure = stderr.fileHandleForReading.readDataToEndOfFile()
            guard process.terminationStatus == 0 else {
                let message = String(decoding: failure.isEmpty ? output : failure, as: UTF8.self)
                    .trimmingCharacters(in: .whitespacesAndNewlines)
                throw RuntimeBridgeError.commandFailed(
                    arguments.joined(separator: " "),
                    message.isEmpty ? "命令没有返回错误详情" : message
                )
            }
            return output
        }.value
    }

    private static func discoverHelper() -> URL {
        let fileManager = FileManager.default
        let environment = ProcessInfo.processInfo.environment
        var candidates = [URL]()
        candidates.append(
            Bundle.main.bundleURL
                .appendingPathComponent("Contents/Helpers/voice-input")
        )
        if let override = environment["VOICE_INPUT_HELPER"], !override.isEmpty {
            candidates.append(URL(fileURLWithPath: override))
        }
        let sourceRoot = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        candidates.append(sourceRoot.appendingPathComponent("target/release/voice-input"))
        candidates.append(sourceRoot.appendingPathComponent("target/debug/voice-input"))
        candidates.append(
            URL(fileURLWithPath: fileManager.currentDirectoryPath)
                .appendingPathComponent("target/debug/voice-input")
        )
        return candidates.first(where: { fileManager.isExecutableFile(atPath: $0.path) })
            ?? candidates[0]
    }
}

struct LLMProbe: Decodable, Equatable {
    let ok: Bool
    let endpoint: String
    let model: String
    let latencyMs: UInt64
    let responseChars: Int
}

enum RuntimeBridgeError: LocalizedError {
    case helperMissing(String)
    case commandFailed(String, String)

    var errorDescription: String? {
        switch self {
        case .helperMissing(let path):
            "找不到 Voice Input 本地运行组件：\(path)"
        case .commandFailed(_, let message):
            message
        }
    }
}
