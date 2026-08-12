#if os(macOS) && NATIVE_PREVIEW
import SwiftUI
import UsageCore

#Preview("Menu · Healthy") {
    UsageMenuView(model: .preview(scenario: .healthy))
        .frame(width: 380, height: 520)
}

#Preview("Menu · Stale") {
    UsageMenuView(model: .preview(scenario: .stale))
        .frame(width: 380, height: 520)
}

#Preview("Menu · Refresh Error") {
    UsageMenuView(model: .preview(scenario: .refreshError))
        .frame(width: 380, height: 520)
}

#Preview("Main · Light") {
    MainWindowView(model: .preview())
        .frame(width: 1000, height: 650)
        .preferredColorScheme(.light)
}

#Preview("Main · Dark") {
    MainWindowView(model: .preview())
        .frame(width: 1000, height: 650)
        .preferredColorScheme(.dark)
}

#Preview("Settings · Fixture Diagnostics") {
    NativeSettingsView(
        model: .preview(),
        initialPane: .diagnostics
    )
    .frame(width: 820, height: 560)
}

#Preview("Settings · Providers") {
    NativeSettingsView(
        model: .preview(),
        initialPane: .providers
    )
    .frame(width: 820, height: 560)
}
#endif
