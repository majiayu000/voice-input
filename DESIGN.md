---
name: Voice Input
description: A quiet, local-first macOS voice input method that feels native to the system.
colors:
  ink: "#22201c"
  muted-ink: "#6f6a63"
  sheet: "#f7f6f3"
  surface: "#ffffff"
  divider: "#dedbd4"
  status-mark: "#1f1d1a"
  recording: "#b42318"
  success: "#2c6a45"
typography:
  title:
    fontFamily: "-apple-system, BlinkMacSystemFont, PingFang SC, sans-serif"
    fontSize: "18px"
    fontWeight: 600
    lineHeight: 1.3
  body:
    fontFamily: "-apple-system, BlinkMacSystemFont, PingFang SC, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.5
  label:
    fontFamily: "-apple-system, BlinkMacSystemFont, PingFang SC, sans-serif"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: 1.4
  status-glyph:
    fontFamily: "Songti SC, STSong, serif"
    fontSize: "12px"
    fontWeight: 600
    lineHeight: 1
  metric:
    fontFamily: "SF Mono, Menlo, monospace"
    fontSize: "10px"
    fontWeight: 400
    lineHeight: 1.4
rounded:
  xs: "2px"
  sm: "4px"
  md: "8px"
  lg: "10px"
spacing:
  xs: "4px"
  sm: "8px"
  md: "12px"
  lg: "16px"
  xl: "24px"
components:
  status-mark:
    backgroundColor: "{colors.status-mark}"
    textColor: "{colors.sheet}"
    typography: "{typography.status-glyph}"
    rounded: "{rounded.xs}"
    width: "22px"
    height: "16px"
  status-mark-recording:
    backgroundColor: "{colors.recording}"
    textColor: "{colors.sheet}"
    typography: "{typography.status-glyph}"
    rounded: "{rounded.xs}"
    width: "22px"
    height: "16px"
  candidate-strip:
    backgroundColor: "{colors.sheet}"
    textColor: "{colors.ink}"
    typography: "{typography.body}"
    rounded: "{rounded.xs}"
    padding: "8px 10px"
  settings-container:
    backgroundColor: "{colors.sheet}"
    textColor: "{colors.ink}"
    typography: "{typography.body}"
    rounded: "{rounded.lg}"
    padding: "22px 24px"
  settings-row:
    backgroundColor: "{colors.sheet}"
    textColor: "{colors.ink}"
    typography: "{typography.body}"
    padding: "9px 0"
  action-button:
    backgroundColor: "{colors.sheet}"
    textColor: "{colors.recording}"
    typography: "{typography.body}"
    rounded: "{rounded.sm}"
    padding: "6px 10px"
---

# Design System: Voice Input

## 1. Overview

**Creative North Star: "The Native Input Method"**

Voice Input should feel like a capability that has always belonged in macOS: quiet in the menu bar, immediate beside the insertion point, and familiar when opened in Settings. Its visual language borrows the restraint and density of macOS input sources rather than the spectacle of a standalone AI product. The interface remains nearly invisible during ordinary use and becomes explicit only when the user must grant permission, choose a model, or recover from an error.

The system rejects the centered floating pill, continuous waveform, and overlay writing-assistant patterns associated with Typeless and Wispr. It also rejects chatbot shells, document editors, large dashboards, decorative glass, gradients, neon accents, marketing-scale metrics, and repetitive card grids. Native controls, readable state labels, and direct recovery actions always take priority over novelty.

**Key Characteristics:**

- Native macOS density and control vocabulary.
- A square “听” status mark and a compact cursor-adjacent candidate strip.
- Restrained color used only to communicate recording, success, and required action.
- System-driven light, dark, increased-contrast, and reduced-motion behavior.
- Plain-language privacy and permission guidance for non-technical users.

## 2. Colors

The palette is warm-neutral and nearly monochrome; color is rare so that recording and recovery states remain unmistakable.

### Primary

- **Recording Red:** Reserved for active recording, destructive consequences, and the single most important recovery action. It must never become a decorative brand wash.

### Secondary

- **Local Success Green:** Used only for completed local work and verified ready states, always paired with a label or icon.

### Neutral

- **Warm Ink:** Primary copy and high-emphasis controls.
- **Muted Stone:** Secondary labels, hints, timestamps, and supporting metadata.
- **Quiet Sheet:** Floating menus, the candidate strip, and settings containers in the light appearance.
- **System Surface:** Content fields and native window surfaces.
- **Hairline:** Dividers and container outlines.
- **Status Mark:** The default square menu-bar input-source mark.

At runtime, SwiftUI semantic colors are the source of truth for system appearance. These tokens define the light reference; dark mode and increased contrast must be derived from macOS semantic foreground, background, separator, and control colors while preserving the same role hierarchy.

### Named Rules

**The Rare Signal Rule.** Recording Red and Local Success Green together should occupy less than ten percent of any screen. Color never carries state alone; pair it with text, iconography, or shape.

## 3. Typography

**Display Font:** System UI / PingFang SC, with the macOS system fallback.
**Body Font:** System UI / PingFang SC, with the macOS system fallback.
**Label/Mono Font:** SF Mono for latency, model filenames, and diagnostic values; Songti SC only for the square “听” status glyph.

**Character:** Typography should read as a native system utility: compact, calm, and immediately legible. The one serif glyph references familiar Chinese input-source marks without turning the rest of the product into a themed interface.

### Hierarchy

- **Title** (600, 18px, 1.3): Onboarding and settings-window section titles.
- **Body** (400, 13px, 1.5): Controls, explanatory copy, and menu content.
- **Label** (400, 12px, 1.4): Secondary state, help text, and metadata.
- **Status Glyph** (600, 12px, 1): The single “听” mark only.
- **Metric** (400, 10px, 1.4): Latency, model identifiers, and diagnostic details.

### Named Rules

**The System Voice Rule.** Use the macOS system type scale and Dynamic Type behavior. Never introduce display typography, all-caps marketing copy, or a second decorative family beyond the single status glyph.

## 4. Elevation

The system is flat by default. Depth comes from native window layering, hairline separators, and macOS materials. A shadow is permitted only for a detached menu, onboarding sheet, or candidate strip that must read above another application; settings rows and ordinary controls remain flat.

### Shadow Vocabulary

- **Detached Utility:** A soft, neutral ambient shadow for menus and transient sheets. Never use it on every row or panel.

### Named Rules

**The Structural Shadow Rule.** Shadows explain physical separation; they are not decoration and do not appear on static settings content.

## 5. Components

Components should follow native macOS sizing, focus, keyboard, and accessibility behavior before applying project-specific styling.

### Buttons

- **Shape:** Native macOS button shape; compact custom text actions may use the small radius.
- **Primary:** One clear action per onboarding step, using the system accent behavior and direct verbs such as “继续” or “打开系统设置”.
- **Hover / Focus:** Use native hover and focus-ring behavior. Reduced Motion removes positional transitions.
- **Secondary / Ghost:** Quiet text actions for back, retry, copy diagnostics, and disclosure.

### Cards / Containers

- **Corner Style:** Medium for menu surfaces and large for onboarding or settings sheets.
- **Background:** Quiet Sheet or the native semantic window background.
- **Shadow Strategy:** Only detached transient surfaces use Detached Utility elevation.
- **Border:** A one-pixel semantic separator or hairline; never a colored stripe.
- **Internal Padding:** Use the documented spacing scale, with 24px reserved for sheet edges.

### Inputs / Fields

- **Style:** Use native toggles, pickers, secure fields, and text fields with macOS control sizing.
- **Focus:** Preserve the native keyboard focus ring and full keyboard navigation.
- **Error / Disabled:** Disabled controls retain readable labels; errors show a plain-language reason and a next action, not color alone.

### Navigation

Settings use a familiar macOS toolbar or sidebar with four destinations: 常规、识别、文本、诊断. The menu bar remains the daily entry point. The selected destination uses native selection treatment and never becomes a decorative pill.

### Status Mark and Candidate Strip

The status mark is a 22-by-16-pixel square carrying “听”. It changes from near-black to Recording Red only while listening or finalizing. The candidate strip appears near the active insertion point, uses a nearly square two-pixel radius, states “正在听 / 正在识别 / 已写入” in words, and disappears promptly after completion. It contains no waveform.

## 6. Do's and Don'ts

### Do:

- **Do** use a square “听” menu-bar mark and a compact cursor-adjacent candidate strip.
- **Do** distinguish process running, service ready, listening, recognizing, completed, blocked, and error states with text.
- **Do** use native macOS controls, keyboard focus, accessibility labels, semantic color, and system appearance.
- **Do** explain microphone, Accessibility, and Input Monitoring permissions in ordinary language and provide the exact next action.
- **Do** keep local processing visible: default to no saved audio and no LLM, with remote features opt-in.

### Don't:

- **Don't** copy Typeless or Wispr's centered floating pill, continuous waveform, or writing-assistant overlay.
- **Don't** turn the product into a chatbot, document editor, or large dashboard.
- **Don't** use decorative glass panels, gradient text, neon color, marketing-scale metrics, or repetitive card grids.
- **Don't** invent controls when macOS settings, menus, buttons, toggles, and permission affordances already exist.
- **Don't** require users to understand ASR models, LaunchAgent, TCC permissions, terminal commands, or other implementation jargon.
