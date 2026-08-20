import XCTest
@testable import VoiceInputApp

final class ModelsTests: XCTestCase {
    func testControlSnapshotDecodesVersionedRustPayload() throws {
        let payload = #"""
        {
          "schema_version": 1,
          "generated_at_ms": 42,
          "service": {
            "installed": true,
            "loaded": true,
            "launchd_state": "running",
            "pid": 123,
            "last_exit_code": 0,
            "runtime": null,
            "paths": {
              "data_dir": "/tmp/data",
              "binary": "/tmp/Voice Input Runtime.app/Contents/MacOS/voice-input",
              "config": "/tmp/config.toml",
              "stdout_log": "/tmp/out.log",
              "stderr_log": "/tmp/err.log",
              "runtime_status": "/tmp/status.json",
              "latency_history": "/tmp/latency.jsonl"
            }
          },
          "settings": {
            "config_path": "/tmp/config.toml",
            "hotkey": "fn",
            "audible_feedback": true,
            "language": "zh",
            "insertion_mode": "auto",
            "active_model_path": "/tmp/model.bin",
            "refiner_enabled": false,
            "refiner_base_url": "http://127.0.0.1:11434/v1",
            "refiner_model": "",
            "refiner_api_key_env": null,
            "refiner_allow_remote": false,
            "refiner_failure_mode": "bypass",
            "refiner_system_prompt": "Polish",
            "vad_enabled": true
          },
          "models": [{
            "preset": "balanced",
            "file_name": "model.bin",
            "size_bytes": 100,
            "description": "balanced",
            "path": "/tmp/model.bin",
            "installed": true,
            "active": true
          }],
          "recent_runtime": null
        }
        """#.data(using: .utf8)!

        let snapshot = try JSONDecoder.voiceInput.decode(ControlSnapshot.self, from: payload)

        XCTAssertEqual(snapshot.schemaVersion, 1)
        XCTAssertEqual(snapshot.settings.hotkey, "fn")
        XCTAssertEqual(snapshot.models.first?.active, true)
        XCTAssertEqual(snapshot.service.paths.stderrLog, "/tmp/err.log")
        XCTAssertEqual(
            snapshot.service.paths.binary,
            "/tmp/Voice Input Runtime.app/Contents/MacOS/voice-input"
        )
    }

    func testPatchUsesRustSnakeCaseKeys() throws {
        let patch = SettingsPatch(hotkey: "fn", audibleFeedback: false)
        let object = try JSONSerialization.jsonObject(with: JSONEncoder.voiceInput.encode(patch)) as! [String: Any]

        XCTAssertEqual(object["hotkey"] as? String, "fn")
        XCTAssertEqual(object["audible_feedback"] as? Bool, false)
        XCTAssertNil(object["language"])
    }

    func testLLMDraftValidationAndPatchTrimming() throws {
        var draft = LLMSettingsDraft()
        draft.enabled = true
        draft.endpoint = "   "
        draft.model = "local-model"
        XCTAssertFalse(draft.isValid)

        draft.endpoint = "  http://127.0.0.1:11434/v1  "
        draft.keyEnvironment = "  LOCAL_LLM_API_KEY  "
        XCTAssertTrue(draft.isValid)

        let object = try JSONSerialization.jsonObject(
            with: JSONEncoder.voiceInput.encode(draft.patch)
        ) as! [String: Any]
        XCTAssertEqual(object["refiner_base_url"] as? String, "http://127.0.0.1:11434/v1")
        XCTAssertEqual(object["refiner_api_key_env"] as? String, "LOCAL_LLM_API_KEY")
    }

    func testLLMDraftRejectsUnsafeRemoteEndpointsBeforeSave() {
        var draft = LLMSettingsDraft()
        draft.enabled = true
        draft.model = "remote-model"
        draft.endpoint = "http://example.com/v1"

        XCTAssertEqual(
            draft.validationMessage,
            "远程地址需要先开启“允许 HTTPS 远程地址”。"
        )
        draft.allowRemote = true
        XCTAssertEqual(draft.validationMessage, "远程服务必须使用 HTTPS。")
        draft.endpoint = "https://example.com/v1"
        XCTAssertNil(draft.validationMessage)
        XCTAssertTrue(draft.isRemoteEndpoint)
    }

    func testDiagnosticExportRedactsTranscriptAndSecrets() throws {
        let payload = #"""
        {
          "recent_runtime": {"last_text": "私密听写 🧪", "last_error": null},
          "settings": {"refiner_api_key_env": "LOCAL_KEY", "api_key": "secret-value"},
          "nested": [{"authorization": "Bearer secret"}]
        }
        """#.data(using: .utf8)!

        let output = try DiagnosticExport.sanitizeJSON(payload)

        XCTAssertFalse(output.contains("私密听写"))
        XCTAssertFalse(output.contains("secret-value"))
        XCTAssertFalse(output.contains("Bearer secret"))
        XCTAssertTrue(output.contains("LOCAL_KEY"))
        XCTAssertTrue(output.contains("<redacted>"))
    }

    func testRuntimeCommandDrainsLargeOutputWithoutPipeDeadlock() throws {
        let data = try RuntimeBridge.runSynchronously(
            ["-c", "dd if=/dev/zero bs=1024 count=512 2>/dev/null | tr '\\0' x"],
            input: nil,
            executable: URL(fileURLWithPath: "/bin/sh"),
            timeout: 3
        )

        XCTAssertEqual(data.count, 512 * 1_024)
    }

    func testRuntimeCommandHasBoundedTimeout() throws {
        XCTAssertThrowsError(
            try RuntimeBridge.runSynchronously(
                ["2"],
                input: nil,
                executable: URL(fileURLWithPath: "/bin/sleep"),
                timeout: 0.05
            )
        ) { error in
            guard case RuntimeBridgeError.commandTimedOut = error else {
                return XCTFail("unexpected error: \(error)")
            }
        }
    }
}
