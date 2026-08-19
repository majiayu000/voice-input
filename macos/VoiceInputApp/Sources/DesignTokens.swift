import SwiftUI

enum VoiceInputDesign {
    static let recording = Color(red: 0.71, green: 0.14, blue: 0.09)
    static let success = Color(red: 0.17, green: 0.42, blue: 0.27)
    static let cornerSmall: CGFloat = 4
    static let cornerMedium: CGFloat = 8
    static let spaceSmall: CGFloat = 8
    static let spaceMedium: CGFloat = 12
    static let spaceLarge: CGFloat = 16
    static let transitionDuration = 0.18

    static var stateAnimation: Animation {
        .timingCurve(0.22, 1, 0.36, 1, duration: transitionDuration)
    }
}
