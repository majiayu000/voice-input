import AppKit
import Darwin
import SwiftUI

final class AppDelegate: NSObject, NSApplicationDelegate {
    private var instanceLockDescriptor: Int32 = -1

    func applicationDidFinishLaunching(_ notification: Notification) {
        if acquireInstanceLock() == false {
            activateExistingInstance()
            exit(EXIT_SUCCESS)
        }
        AppModel.shared.start()
    }

    func applicationWillTerminate(_ notification: Notification) {
        guard instanceLockDescriptor >= 0 else { return }
        flock(instanceLockDescriptor, LOCK_UN)
        close(instanceLockDescriptor)
        instanceLockDescriptor = -1
    }

    private func acquireInstanceLock() -> Bool? {
        guard let applicationSupport = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first else { return nil }
        let directory = applicationSupport.appendingPathComponent("voice-input", isDirectory: true)
        do {
            try FileManager.default.createDirectory(
                at: directory,
                withIntermediateDirectories: true
            )
        } catch {
            return nil
        }
        let descriptor = open(
            directory.appendingPathComponent("gui.lock").path,
            O_CREAT | O_RDWR | O_CLOEXEC,
            S_IRUSR | S_IWUSR
        )
        guard descriptor >= 0 else { return nil }
        guard flock(descriptor, LOCK_EX | LOCK_NB) == 0 else {
            close(descriptor)
            return false
        }
        instanceLockDescriptor = descriptor
        return true
    }

    private func activateExistingInstance() {
        guard let identifier = Bundle.main.bundleIdentifier,
              let existing = NSRunningApplication
                .runningApplications(withBundleIdentifier: identifier)
                .first(where: { $0.processIdentifier != ProcessInfo.processInfo.processIdentifier })
        else { return }
        existing.activate(options: [])
    }
}

@main
struct VoiceInputApplication: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @StateObject private var model = AppModel.shared

    var body: some Scene {
        MenuBarExtra {
            MenuBarPanel(model: model)
        } label: {
            MenuBarLabel(model: model)
        }
        .menuBarExtraStyle(.window)

        Settings {
            SettingsRootView(model: model)
        }
        .windowResizability(.contentSize)

        .commands {
            CommandGroup(after: .appSettings) {
                Button("复制最近一次文字") {
                    model.copyRecentText()
                }
                .keyboardShortcut("c", modifiers: [.command, .shift])
                .disabled(model.activeRuntime?.lastText?.isEmpty != false)

                Button("重新查看使用引导") {
                    model.showOnboarding()
                }
                .keyboardShortcut("?", modifiers: [.command, .shift])
            }
        }
    }
}
