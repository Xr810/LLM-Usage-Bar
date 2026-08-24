#if os(macOS)
import Foundation
import UsageCore

/// 手动粘贴模型清单的解析。
///
/// 决定 11:**手动是保底,不是降级** —— 有些 provider 永远拉不到 `/v1/models`,
/// 手动这条路必须一直好用。所以解析要宽容:用户从各家控制台复制出来的东西
/// 长什么样都有,不该逼他先手工整理成某种格式。
public enum ModelRoutePaste {
    /// 接受的写法(每行一条,空行与 `#` 开头的注释行忽略):
    ///
    ///     gpt-5.6 = sol-gpt-5.6-1120     等号
    ///     gpt-5.6 -> sol-gpt-5.6-1120    箭头(-> 或 →)
    ///     gpt-5.6 : sol-gpt-5.6-1120     冒号
    ///     gpt-5.6, sol-gpt-5.6-1120      逗号
    ///     gpt-5.6 <TAB> sol-gpt-5.6-1120 制表符
    ///     gpt-5.6                        只有一个名字 → 上游同名
    ///
    /// 后者是常见情形:很多中转的模型 ID 就和官方一样,用户直接把清单贴进来即可。
    public static func parse(_ text: String) -> [ModelRouteInputV1] {
        var seen = Set<String>()
        var routes: [ModelRouteInputV1] = []

        for rawLine in text.split(separator: "\n", omittingEmptySubsequences: false) {
            let line = rawLine.trimmingCharacters(in: .whitespaces)
            if line.isEmpty || line.hasPrefix("#") { continue }

            let (left, right) = split(line)
            let logical = left.trimmingCharacters(in: separatorsAndSpace)
            guard !logical.isEmpty else { continue }

            let upstreamRaw = right?.trimmingCharacters(in: separatorsAndSpace) ?? ""
            let upstream = upstreamRaw.isEmpty ? logical : upstreamRaw

            // 同一个逻辑模型只保留第一条 —— 后面的重复行是用户复制时带出来的噪音,
            // 静默覆盖会让他以为自己写的那条生效了。
            guard seen.insert(logical).inserted else { continue }
            routes.append(ModelRouteInputV1(logicalModel: logical, upstreamModel: upstream))
        }
        return routes
    }

    private static let separatorsAndSpace = CharacterSet(charactersIn: " \t=:,>-→\"'")

    private static func split(_ line: String) -> (String, String?) {
        // 按最先出现的分隔符切一刀。多字符的 `->` 要先于单字符的 `-` 判断。
        for token in ["->", "→", "=", ":", ",", "\t"] {
            if let range = line.range(of: token) {
                return (String(line[line.startIndex..<range.lowerBound]),
                        String(line[range.upperBound...]))
            }
        }
        // 没有分隔符时,退到「空白切一刀」;整行只有一个词就当同名映射。
        let parts = line.split(separator: " ", maxSplits: 1, omittingEmptySubsequences: true)
        if parts.count == 2 { return (String(parts[0]), String(parts[1])) }
        return (line, nil)
    }
}
#endif
