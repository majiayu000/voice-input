import Foundation
import Darwin

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
        _ = try await run(["install"], timeout: 30)
    }

    func startService() async throws {
        _ = try await run(["start"], timeout: 30)
    }

    func stopService() async throws {
        _ = try await run(["stop"], timeout: 15)
    }

    func installModel(_ preset: String) async throws {
        _ = try await run(["model", "install", "--preset", preset], timeout: 3_600)
    }

    func apply(_ patch: SettingsPatch) async throws {
        let input = try JSONEncoder.voiceInput.encode(patch)
        _ = try await run(["control", "apply"], input: input)
    }

    func testRefiner() async throws -> LLMProbe {
        let data = try await run(["control", "test-refiner"])
        return try JSONDecoder.voiceInput.decode(LLMProbe.self, from: data)
    }

    func permissionSnapshot(helperPath: String) async throws -> PermissionStatusSnapshot {
        let data = try await runPermissionJob(
            ["permission", "snapshot"],
            helperPath: helperPath,
            timeout: .seconds(8)
        )
        return try JSONDecoder.voiceInput.decode(PermissionStatusSnapshot.self, from: data)
    }

    func requestPermission(
        _ permission: PermissionKind,
        helperPath: String
    ) async throws -> PermissionStatusSnapshot {
        let identifier = UUID().uuidString.lowercased()
        let marker = FileManager.default.temporaryDirectory
            .appendingPathComponent("voice-input-permission-\(identifier).ready")
        defer { try? FileManager.default.removeItem(at: marker) }

        let application = URL(fileURLWithPath: helperPath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        _ = try await run(
            [
                "-n", "-g", application.path, "--args",
                "permission", "request", permission.cliArgument,
                "--ready-file", marker.path,
                "--hold-seconds", permission == .microphone ? "0" : "120"
            ],
            executable: URL(fileURLWithPath: "/usr/bin/open")
        )

        let clock = ContinuousClock()
        let markerDeadline = clock.now.advanced(by: .seconds(5))
        while clock.now < markerDeadline {
            if FileManager.default.fileExists(atPath: marker.path) { break }
            try await Task.sleep(for: .milliseconds(100))
        }
        guard FileManager.default.fileExists(atPath: marker.path) else {
            throw RuntimeBridgeError.commandFailed(
                "permission request \(permission.cliArgument)",
                "Voice Input Runtime 未能启动权限请求"
            )
        }

        if permission == .microphone {
            let responseDeadline = clock.now.advanced(by: .seconds(20))
            while clock.now < responseDeadline {
                let snapshot = try await permissionSnapshot(helperPath: helperPath)
                if snapshot.microphone != .notDetermined { return snapshot }
                try await Task.sleep(for: .milliseconds(250))
            }
        } else {
            try await Task.sleep(for: .milliseconds(500))
        }
        return try await permissionSnapshot(helperPath: helperPath)
    }

    private func runPermissionJob(
        _ arguments: [String],
        helperPath: String,
        timeout: Duration
    ) async throws -> Data {
        let identifier = UUID().uuidString.lowercased()
        let label = "com.starlight.voiceinput.permission.\(identifier)"
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent(label, isDirectory: true)
        let output = directory.appendingPathComponent("stdout.json")
        let failure = directory.appendingPathComponent("stderr.log")
        try FileManager.default.createDirectory(
            at: directory,
            withIntermediateDirectories: true
        )
        defer {
            Self.removeLaunchJob(label)
            try? FileManager.default.removeItem(at: directory)
        }

        _ = try await run(
            [
                "submit",
                "-l", label,
                "-o", output.path,
                "-e", failure.path,
                "--", helperPath
            ] + arguments,
            executable: URL(fileURLWithPath: "/bin/launchctl")
        )

        let clock = ContinuousClock()
        let deadline = clock.now.advanced(by: timeout)
        while clock.now < deadline {
            if let data = try? Data(contentsOf: output), !data.isEmpty {
                return data
            }
            if let data = try? Data(contentsOf: failure), !data.isEmpty {
                let message = String(decoding: data, as: UTF8.self)
                    .trimmingCharacters(in: .whitespacesAndNewlines)
                throw RuntimeBridgeError.commandFailed(
                    arguments.joined(separator: " "),
                    message
                )
            }
            try await Task.sleep(for: .milliseconds(100))
        }
        throw RuntimeBridgeError.commandFailed(
            arguments.joined(separator: " "),
            "运行组件权限请求超时"
        )
    }

    func diagnosticSnapshot() async throws -> String {
        let data = try await run(["control", "snapshot"])
        return try DiagnosticExport.sanitizeJSON(data)
    }

    func runtimeSnapshot(at path: String) throws -> RuntimeSnapshot? {
        let url = URL(fileURLWithPath: path)
        guard FileManager.default.fileExists(atPath: url.path) else { return nil }
        let data = try Data(contentsOf: url, options: .mappedIfSafe)
        return try JSONDecoder.voiceInput.decode(RuntimeSnapshot.self, from: data)
    }

    private func run(
        _ arguments: [String],
        input: Data? = nil,
        executable: URL? = nil,
        timeout: TimeInterval = 15
    ) async throws -> Data {
        let executable = executable ?? helperURL
        return try await Task.detached(priority: .userInitiated) {
            try Self.runSynchronously(
                arguments,
                input: input,
                executable: executable,
                timeout: timeout
            )
        }.value
    }

    nonisolated static func runSynchronously(
        _ arguments: [String],
        input: Data?,
        executable: URL,
        timeout: TimeInterval
    ) throws -> Data {
        guard FileManager.default.isExecutableFile(atPath: executable.path) else {
            throw RuntimeBridgeError.helperMissing(executable.path)
        }

        let fileManager = FileManager.default
        let temporaryDirectory = fileManager.temporaryDirectory
            .appendingPathComponent("voice-input-command-\(UUID().uuidString)", isDirectory: true)
        try fileManager.createDirectory(at: temporaryDirectory, withIntermediateDirectories: true)
        defer { try? fileManager.removeItem(at: temporaryDirectory) }

        let stdoutURL = temporaryDirectory.appendingPathComponent("stdout")
        let stderrURL = temporaryDirectory.appendingPathComponent("stderr")
        guard fileManager.createFile(atPath: stdoutURL.path, contents: nil),
              fileManager.createFile(atPath: stderrURL.path, contents: nil) else {
            throw RuntimeBridgeError.temporaryFilesUnavailable
        }
        let stdout = try FileHandle(forWritingTo: stdoutURL)
        let stderr = try FileHandle(forWritingTo: stderrURL)

        let process = Process()
        process.executableURL = executable
        process.arguments = arguments
        process.standardOutput = stdout
        process.standardError = stderr

        var stdin: FileHandle?
        if let input {
            let stdinURL = temporaryDirectory.appendingPathComponent("stdin")
            try input.write(to: stdinURL, options: .atomic)
            stdin = try FileHandle(forReadingFrom: stdinURL)
            process.standardInput = stdin
        }

        let finished = DispatchSemaphore(value: 0)
        process.terminationHandler = { _ in finished.signal() }
        try process.run()
        if finished.wait(timeout: .now() + timeout) == .timedOut {
            process.terminate()
            if finished.wait(timeout: .now() + 2) == .timedOut {
                kill(process.processIdentifier, SIGKILL)
                _ = finished.wait(timeout: .now() + 1)
            }
            throw RuntimeBridgeError.commandTimedOut(
                arguments.joined(separator: " "),
                timeout
            )
        }

        try? stdin?.close()
        try stdout.close()
        try stderr.close()
        let output = try Data(contentsOf: stdoutURL)
        let failure = try Data(contentsOf: stderrURL)
        guard process.terminationStatus == 0 else {
            let message = String(decoding: failure.isEmpty ? output : failure, as: UTF8.self)
                .trimmingCharacters(in: .whitespacesAndNewlines)
            throw RuntimeBridgeError.commandFailed(
                arguments.joined(separator: " "),
                message.isEmpty ? "命令没有返回错误详情" : message
            )
        }
        return output
    }

    private nonisolated static func removeLaunchJob(_ label: String) {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/launchctl")
        process.arguments = ["remove", label]
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        try? process.run()
        process.waitUntilExit()
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
    case commandTimedOut(String, TimeInterval)
    case permissionStatusUnavailable(String)
    case temporaryFilesUnavailable

    var errorDescription: String? {
        switch self {
        case .helperMissing(let path):
            "找不到 Voice Input 本地运行组件：\(path)"
        case .commandFailed(_, let message):
            message
        case .commandTimedOut(let command, let seconds):
            "命令 \(command) 等待超时（\(Int(seconds)) 秒）。"
        case .permissionStatusUnavailable(let message):
            "无法读取运行组件的权限状态：\(message)"
        case .temporaryFilesUnavailable:
            "无法创建本地命令输出文件。"
        }
    }
}
