#if os(macOS)
import AppKit
import SwiftUI
import UsageCore

@main
struct LLMUsageBarNativeApp: App {
    @StateObject private var model = UsageMenuModel()

    var body: some Scene {
        MenuBarExtra {
            UsageMenuView(model: model)
                .frame(width: 360)
        } label: {
            Image(systemName: model.snapshot.health.symbolName)
                .accessibilityLabel("LLM usage: \(model.snapshot.health.rawValue)")
        }
        .menuBarExtraStyle(.window)
    }
}

@MainActor
final class UsageMenuModel: ObservableObject {
    @Published private(set) var snapshot = UsageSnapshot(generatedAt: .now, providers: [])
    @Published private(set) var errorMessage: String?
    @Published private(set) var isRefreshing = false

    init() {
        Task { await refresh() }
    }

    func refresh() async {
        isRefreshing = true
        defer { isRefreshing = false }
        do {
            let provider = try await Task.detached { try ClaudeStatuslineSource().load() }.value
            snapshot = UsageSnapshot(generatedAt: .now, providers: [provider])
            errorMessage = nil
        } catch {
            snapshot = UsageSnapshot(generatedAt: .now, providers: [])
            errorMessage = "Claude usage is unavailable. Start a Claude Code session with the LLM Usage Bar status-line bridge enabled."
        }
    }
}

private struct UsageMenuView: View {
    let model: UsageMenuModel

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("LLM Usage Bar")
                .font(.headline)

            if let errorMessage = model.errorMessage {
                VStack(spacing: 8) {
                    Image(systemName: "chart.bar.xaxis")
                        .font(.title2)
                    Text("Waiting for usage data")
                        .font(.subheadline.weight(.semibold))
                    Text(errorMessage)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 12)
            }

            ForEach(model.snapshot.providers) { provider in
                VStack(alignment: .leading, spacing: 8) {
                    Label(provider.name, systemImage: provider.health.symbolName)
                        .font(.subheadline.weight(.semibold))

                    ForEach(provider.windows) { window in
                        HStack {
                            Text(window.title)
                            Spacer()
                            Text(window.remainingPercent.map { "\(Int($0.rounded()))% left" } ?? "Waiting for data")
                                .foregroundStyle(.secondary)
                        }
                        ProgressView(value: window.utilizationPercent ?? 0, total: 100)
                    }
                }
                .padding(12)
                .background(.quaternary, in: RoundedRectangle(cornerRadius: 10))
            }

            Divider()
            HStack {
                Button("Refresh") { Task { await model.refresh() } }
                    .disabled(model.isRefreshing)
                Spacer()
                Button("Quit") { NSApplication.shared.terminate(nil) }
                    .keyboardShortcut("q")
            }
        }
        .padding(16)
    }
}
#else
import Foundation

@main
enum LLMUsageBarNativeApp {
    static func main() {
        print("LLMUsageBarNative requires macOS 13 or later.")
    }
}
#endif
