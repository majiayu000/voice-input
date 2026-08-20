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
}
