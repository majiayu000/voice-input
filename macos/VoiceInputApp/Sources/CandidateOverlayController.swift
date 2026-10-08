import AppKit
import ApplicationServices
import QuartzCore
import SwiftUI

enum CandidateState: Equatable {
    case listening
    case recognizing
    case completed

    var title: String {
        switch self {
        case .listening: "正在听"
        case .recognizing: "正在处理听写"
        case .completed: "已写入"
        }
    }

    func detail(hotkey: String?) -> String {
        switch self {
        case .listening:
            "本机 · 松开 \(hotkey == "fn" ? "Fn" : "Control + Shift + Space") 写入"
        case .recognizing: "正在整理文字"
        case .completed: "文字已到当前光标"
        }
    }

    var color: Color {
        switch self {
        case .listening, .recognizing: VoiceInputDesign.recording
        case .completed: VoiceInputDesign.success
        }
    }
}

@MainActor
final class CandidateOverlayController {
    static let shared = CandidateOverlayController()

    private var panel: NSPanel?
    private var dismissTask: Task<Void, Never>?

    func update(runtime: RuntimeSnapshot?, completedNow: Bool) {
        if completedNow {
            show(.completed, runtime: runtime)
            dismissTask?.cancel()
            dismissTask = Task { [weak self] in
                try? await Task.sleep(for: .seconds(1.1))
                self?.hide()
            }
            return
        }
        switch runtime?.phase {
        case "capturing": show(.listening, runtime: runtime)
        case "finalizing": show(.recognizing, runtime: runtime)
        default:
            if panel?.isVisible == true, dismissTask == nil { hide() }
        }
    }

    private func show(_ state: CandidateState, runtime: RuntimeSnapshot?) {
        dismissTask?.cancel()
        dismissTask = nil
        let size = NSSize(width: 286, height: 48)
        let panel = panel ?? makePanel(size: size)
        let elapsed = runtime.map { max(0, Int(Date().timeIntervalSince1970 * 1_000) - Int($0.updatedAtMs)) }
        panel.contentView = NSHostingView(rootView: CandidateStrip(
            state: state,
            hotkey: runtime?.hotkey,
            elapsedMilliseconds: elapsed,
            completedText: state == .completed ? runtime?.lastText : nil
        ))
        let origin = focusedInsertionOrigin(panelSize: size)
        panel.setFrame(NSRect(origin: origin, size: size), display: true)
        if panel.isVisible {
            panel.orderFrontRegardless()
        } else if NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
            panel.orderFrontRegardless()
        } else {
            panel.alphaValue = 0
            panel.orderFrontRegardless()
            NSAnimationContext.runAnimationGroup { context in
                context.duration = VoiceInputDesign.transitionDuration
                context.timingFunction = CAMediaTimingFunction(controlPoints: 0.22, 1, 0.36, 1)
                panel.animator().alphaValue = 1
            }
        }
        self.panel = panel
    }

    private func hide() {
        guard let panel else { return }
        if NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
            panel.orderOut(nil)
        } else {
            NSAnimationContext.runAnimationGroup { context in
                context.duration = 0.13
                context.timingFunction = CAMediaTimingFunction(controlPoints: 0.7, 0, 0.84, 0)
                panel.animator().alphaValue = 0
            } completionHandler: {
                panel.orderOut(nil)
                panel.alphaValue = 1
            }
        }
        dismissTask = nil
    }

    private func makePanel(size: NSSize) -> NSPanel {
        let panel = NSPanel(
            contentRect: NSRect(origin: .zero, size: size),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        panel.level = .statusBar
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = true
        panel.hidesOnDeactivate = false
        panel.ignoresMouseEvents = true
        panel.isReleasedWhenClosed = false
        return panel
    }

    private func focusedInsertionOrigin(panelSize: NSSize) -> NSPoint {
        guard let rect = focusedInsertionRect(), let screen = screen(containing: rect) else {
            let mouse = NSEvent.mouseLocation
            return NSPoint(x: mouse.x + 10, y: mouse.y - panelSize.height - 8)
        }
        let x = min(max(rect.minX, screen.visibleFrame.minX + 8), screen.visibleFrame.maxX - panelSize.width - 8)
        let convertedY = screen.frame.maxY - rect.maxY - panelSize.height - 8
        let y = min(max(convertedY, screen.visibleFrame.minY + 8), screen.visibleFrame.maxY - panelSize.height - 8)
        return NSPoint(x: x, y: y)
    }

    private func focusedInsertionRect() -> CGRect? {
        let system = AXUIElementCreateSystemWide()
        var focusedValue: CFTypeRef?
        guard AXUIElementCopyAttributeValue(
            system,
            kAXFocusedUIElementAttribute as CFString,
            &focusedValue
        ) == .success,
        let focusedValue else { return nil }
        let focused = unsafeBitCast(focusedValue, to: AXUIElement.self)

        var rangeValue: CFTypeRef?
        guard AXUIElementCopyAttributeValue(
            focused,
            kAXSelectedTextRangeAttribute as CFString,
            &rangeValue
        ) == .success,
        let rangeValue else { return nil }

        var boundsValue: CFTypeRef?
        guard AXUIElementCopyParameterizedAttributeValue(
            focused,
            kAXBoundsForRangeParameterizedAttribute as CFString,
            rangeValue,
            &boundsValue
        ) == .success,
        let boundsValue,
        CFGetTypeID(boundsValue) == AXValueGetTypeID() else { return nil }

        var rect = CGRect.zero
        let value = unsafeBitCast(boundsValue, to: AXValue.self)
        guard AXValueGetValue(value, .cgRect, &rect) else { return nil }
        return rect
    }

    private func screen(containing rect: CGRect) -> NSScreen? {
        NSScreen.screens.first { screen in
            let point = NSPoint(x: rect.midX, y: screen.frame.maxY - rect.midY)
            return screen.frame.contains(point)
        } ?? NSScreen.main
    }
}

private struct CandidateStrip: View {
    let state: CandidateState
    let hotkey: String?
    let elapsedMilliseconds: Int?
    let completedText: String?

    var body: some View {
        HStack(spacing: 10) {
            StatusMark(recording: state != .completed)
            VStack(alignment: .leading, spacing: 1) {
                Text(state.title)
                    .font(.system(size: 13, weight: .semibold))
                Text(completedText.map { "“\($0)”" } ?? state.detail(hotkey: hotkey))
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 8)
            if state == .completed {
                Image(systemName: "checkmark")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(state.color)
                    .accessibilityHidden(true)
            } else {
                Text(elapsedLabel)
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.horizontal, 10)
        .frame(width: 286, height: 48)
        .background(.regularMaterial)
        .overlay(Rectangle().stroke(state.color.opacity(0.9), lineWidth: 1))
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(state.title)，\(state.detail(hotkey: hotkey))")
    }

    private var elapsedLabel: String {
        let seconds = Double(elapsedMilliseconds ?? 0) / 1_000
        return String(format: "%.1f", seconds)
    }
}
