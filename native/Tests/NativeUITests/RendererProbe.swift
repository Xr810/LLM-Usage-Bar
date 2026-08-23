#if os(macOS)
import AppKit
import SwiftUI
import Testing

/// 探针:判断 ImageRenderer 在这个环境里到底能不能渲出内容。
@MainActor
@Test func imageRendererProbe() throws {
    guard let dir = ProcessInfo.processInfo.environment["PANEL_SHOTS_DIR"], !dir.isEmpty
    else { return }

    func shoot(_ name: String, _ view: some View) throws {
        let renderer = ImageRenderer(content: view)
        renderer.scale = 2
        guard let image = renderer.nsImage,
              let tiff = image.tiffRepresentation,
              let rep = NSBitmapImageRep(data: tiff),
              let png = rep.representation(using: .png, properties: [:])
        else { print("probe \(name): 渲染返回 nil"); return }
        // 统计非背景像素,不用眼睛也能判断有没有内容
        var nonBackground = 0
        let w = rep.pixelsWide, h = rep.pixelsHigh
        for y in stride(from: 0, to: h, by: 4) {
            for x in stride(from: 0, to: w, by: 4) {
                if let c = rep.colorAt(x: x, y: y), c.brightnessComponent < 0.85 {
                    nonBackground += 1
                }
            }
        }
        try png.write(to: URL(fileURLWithPath: dir).appendingPathComponent("\(name).png"))
        print("probe \(name): \(w)x\(h), 非背景采样点 \(nonBackground)")
    }

    try shoot("probe-a-text", Text("Hello 路由面板").font(.title).padding(40).background(.white))
    try shoot("probe-b-vstack", VStack(alignment: .leading, spacing: 8) {
        Text("官方 ChatGPT").font(.headline)
        Text("chatgpt.com/backend-api").font(.callout).foregroundStyle(.secondary)
        HStack { Image(systemName: "checkmark.circle.fill"); Text("已登录") }
    }.padding(24).frame(width: 400).background(.white))
    try shoot("probe-c-scrollview", ScrollView {
        VStack { ForEach(0..<6) { i in Text("行 \(i)").padding(8) } }
    }.frame(width: 400, height: 300).background(.white))
}
#endif
