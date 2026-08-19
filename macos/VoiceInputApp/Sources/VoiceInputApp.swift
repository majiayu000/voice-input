import AppKit
import SwiftUI

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        AppModel.shared.start()
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
