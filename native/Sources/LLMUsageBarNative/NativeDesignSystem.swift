#if os(macOS)
import AppKit
import SwiftUI
import UsageCore

enum NativePalette {
    static let primaryLight = Color(red: 0.35, green: 0.31, blue: 0.87)
    static let primaryDark = Color(red: 0.41, green: 0.45, blue: 0.95)

    static func primary(_ scheme: ColorScheme) -> Color {
        scheme == .dark ? primaryDark : primaryLight
    }

    static func background(_ scheme: ColorScheme) -> Color {
        scheme == .dark
            ? Color(red: 0.066, green: 0.066, blue: 0.075)
            : Color(red: 0.973, green: 0.969, blue: 0.957)
    }

    static func card(_ scheme: ColorScheme) -> Color {
        scheme == .dark
            ? Color(red: 0.105, green: 0.105, blue: 0.125)
            : Color.white
    }

    static func recessed(_ scheme: ColorScheme) -> Color {
        scheme == .dark ? Color.white.opacity(0.045) : Color.black.opacity(0.035)
    }

    static func border(_ scheme: ColorScheme, highContrast: Bool = false) -> Color {
        if highContrast {
            return scheme == .dark ? Color.white.opacity(0.42) : Color.black.opacity(0.32)
        }
        return scheme == .dark ? Color.white.opacity(0.11) : Color.black.opacity(0.10)
    }

    static func status(_ status: TrayUsageStatus, scheme: ColorScheme) -> Color {
        switch (status, scheme) {
        case (.green, .dark): Color(red: 0.23, green: 0.78, blue: 0.55)
        case (.green, _): Color(red: 0.08, green: 0.50, blue: 0.34)
        case (.yellow, .dark): Color(red: 0.98, green: 0.78, blue: 0.22)
        case (.yellow, _): Color(red: 0.82, green: 0.40, blue: 0.04)
        case (.red, .dark): Color(red: 0.96, green: 0.37, blue: 0.39)
        case (.red, _): Color(red: 0.86, green: 0.15, blue: 0.16)
        case (.unknown, _): Color.secondary
        }
    }
}

struct NativeWindowBackground: View {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    var body: some View {
        ZStack {
            NativePalette.background(colorScheme)
            if !reduceTransparency {
                LinearGradient(
                    colors: [Color.white.opacity(colorScheme == .dark ? 0.035 : 0.18), .clear],
                    startPoint: .top,
                    endPoint: .center
                )
            }
        }
        .ignoresSafeArea()
    }
}

struct NativeCard<Content: View>: View {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.colorSchemeContrast) private var contrast
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    private let padding: CGFloat
    private let content: Content

    init(padding: CGFloat = 16, @ViewBuilder content: () -> Content) {
        self.padding = padding
        self.content = content()
    }

    var body: some View {
        content
            .padding(padding)
            .background {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .fill(reduceTransparency ? AnyShapeStyle(NativePalette.card(colorScheme)) : AnyShapeStyle(.regularMaterial))
                    .overlay {
                        if !reduceTransparency {
                            RoundedRectangle(cornerRadius: 12, style: .continuous)
                                .fill(NativePalette.card(colorScheme).opacity(colorScheme == .dark ? 0.36 : 0.46))
                        }
                    }
            }
            .overlay {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .stroke(
                        NativePalette.border(colorScheme, highContrast: contrast == .increased),
                        lineWidth: contrast == .increased ? 1.2 : 0.75
                    )
            }
            .shadow(
                color: reduceTransparency ? .clear : .black.opacity(colorScheme == .dark ? 0.20 : 0.06),
                radius: 15,
                y: 6
            )
    }
}

struct NativeRecessedSurface<Content: View>: View {
    @Environment(\.colorScheme) private var colorScheme
    private let content: Content

    init(@ViewBuilder content: () -> Content) {
        self.content = content()
    }

    var body: some View {
        content
            .padding(10)
            .background(
                NativePalette.recessed(colorScheme),
                in: RoundedRectangle(cornerRadius: 8, style: .continuous)
            )
    }
}

struct NativeBadge: View {
    @Environment(\.colorScheme) private var colorScheme
    let text: String
    var status: TrayUsageStatus?

    var body: some View {
        HStack(spacing: 5) {
            if let status {
                Circle()
                    .fill(NativePalette.status(status, scheme: colorScheme))
                    .frame(width: 6, height: 6)
            }
            Text(text)
                .lineLimit(1)
        }
        .font(.system(size: 10, weight: .semibold))
        .foregroundStyle(status.map { NativePalette.status($0, scheme: colorScheme) } ?? Color.secondary)
        .padding(.horizontal, 7)
        .padding(.vertical, 3)
        .background(NativePalette.recessed(colorScheme), in: Capsule())
        .accessibilityElement(children: .combine)
    }
}

struct NativeStatusBadge: View {
    let status: TrayUsageStatus

    var body: some View {
        NativeBadge(text: L10n.status(status), status: status)
    }
}

struct NativeProviderIcon: View {
    @Environment(\.colorScheme) private var colorScheme

    let systemPresetKey: String?
    let productGroupId: String
    let name: String
    var size: CGFloat = 30

    var body: some View {
        Group {
            if let image = bundledImage {
                Image(nsImage: image)
                    .resizable()
                    .scaledToFit()
                    .padding(size * 0.17)
            } else {
                Text(initials)
                    .font(.system(size: max(10, size * 0.34), weight: .bold, design: .rounded))
                    .foregroundStyle(NativePalette.primary(colorScheme))
            }
        }
        .frame(width: size, height: size)
        .background(NativePalette.recessed(colorScheme), in: RoundedRectangle(cornerRadius: size * 0.28, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: size * 0.28, style: .continuous)
                .stroke(NativePalette.border(colorScheme), lineWidth: 0.6)
        }
        .accessibilityLabel(name)
    }

    private var bundledImage: NSImage? {
        guard let assetName,
              let url = Bundle.main.url(forResource: assetName, withExtension: "svg")
        else { return nil }
        return NSImage(contentsOf: url)
    }

    private var assetName: String? {
        let candidates = [systemPresetKey, productGroupId, name]
            .compactMap { $0?.lowercased() }
            .joined(separator: " ")
        if candidates.contains("openrouter") { return "openrouter" }
        if candidates.contains("claude") { return "claude" }
        if candidates.contains("anthropic") { return "anthropic" }
        if candidates.contains("openai") || candidates.contains("codex") || candidates.contains("chatgpt") {
            return "openai"
        }
        return nil
    }

    private var initials: String {
        let parts = name.split(whereSeparator: { $0 == " " || $0 == "·" || $0 == "-" })
        let value = parts.prefix(2).compactMap(\.first).map(String.init).joined()
        return value.isEmpty ? "LL" : value.uppercased()
    }
}

struct NativeSectionHeading: View {
    let title: String
    var count: Int?

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 7) {
            Text(title.uppercased())
                .font(.system(size: 10, weight: .semibold))
                .tracking(0.8)
                .foregroundStyle(.secondary)
            if let count {
                Text(count.formatted())
                    .font(.system(size: 10).monospacedDigit())
                    .foregroundStyle(.tertiary)
            }
        }
    }
}
#endif
