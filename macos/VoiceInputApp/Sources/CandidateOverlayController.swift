import AppKit
import ApplicationServices
import SwiftUI

enum CandidateState: Equatable {
    case listening
    case recognizing
    case completed

    var title: String {
        switch self {
        case .listening: "正在听"
        case .recognizing: "正在本机识别"
        case .completed: "已写入"
        }
    }

    var detail: String {
        switch self {
        case .listening: "本机 · 松开 Fn 写入"
        case .recognizing: "正在整理文字"
        case .completed: "文字已到当前光标"
        }
    }

    var color: Color {
        switch self {
        case .listening, .recognizing: Color(red: 0.71, green: 0.14, blue: 0.09)
        case .completed: Color(red: 0.17, green: 0.42, blue: 0.27)
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
            show(.completed)
            dismissTask?.cancel()
            dismissTask = Task { [weak self] in
                try? await Task.sleep(for: .seconds(1.1))
                self?.hide()
            }
            return
        }
        switch runtime?.phase {
        case "capturing": show(.listening)
        case "finalizing": show(.recognizing)
        default:
            if panel?.isVisible == true, dismissTask == nil { hide() }
        }
    }

    private func show(_ state: CandidateState) {
        dismissTask?.cancel()
        dismissTask = nil
        let size = NSSize(width: 286, height: 48)
        let panel = panel ?? makePanel(size: size)
        panel.contentView = NSHostingView(rootView: CandidateStrip(state: state))
        let origin = focusedInsertionOrigin(panelSize: size)
        panel.setFrame(NSRect(origin: origin, size: size), display: true)
        panel.orderFrontRegardless()
        self.panel = panel
    }

    private func hide() {
        panel?.orderOut(nil)
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

    var body: some View {
        HStack(spacing: 10) {
            StatusMark(recording: state != .completed)
            VStack(alignment: .leading, spacing: 1) {
                Text(state.title)
                    .font(.system(size: 13, weight: .semibold))
                Text(state.detail)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 8)
            Image(systemName: state == .completed ? "checkmark" : "ellipsis")
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(state.color)
                .accessibilityHidden(true)
        }
        .padding(.horizontal, 10)
        .frame(width: 286, height: 48)
        .background(.regularMaterial)
        .overlay(Rectangle().stroke(state.color.opacity(0.9), lineWidth: 1))
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(state.title)，\(state.detail)")
    }
}
