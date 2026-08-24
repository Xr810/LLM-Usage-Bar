#if os(macOS)
import Testing
import UsageCore
@testable import NativeUI

@Test func pasteAcceptsEveryCommonSeparator() {
    let routes = ModelRoutePaste.parse("""
        gpt-5.6 = sol-gpt-5.6-1120
        gpt-5.5 -> sol-gpt-5.5
        gpt-5.4 → sol-gpt-5.4
        gpt-5.3 : sol-gpt-5.3
        gpt-5.2, sol-gpt-5.2
        gpt-5.1\tsol-gpt-5.1
        gpt-5.0 sol-gpt-5.0
        """)

    #expect(routes.map(\.logicalModel)
        == ["gpt-5.6", "gpt-5.5", "gpt-5.4", "gpt-5.3", "gpt-5.2", "gpt-5.1", "gpt-5.0"])
    #expect(routes.map(\.upstreamModel) == [
        "sol-gpt-5.6-1120", "sol-gpt-5.5", "sol-gpt-5.4",
        "sol-gpt-5.3", "sol-gpt-5.2", "sol-gpt-5.1", "sol-gpt-5.0",
    ])
}

@Test func bareModelNamesMapToThemselves() {
    // 很多中转的模型 ID 就和官方一样,用户直接把清单贴进来就该能用。
    let routes = ModelRoutePaste.parse("""
        gpt-5.6
        gpt-5.6-sol
        """)
    #expect(routes == [
        ModelRouteInputV1(logicalModel: "gpt-5.6", upstreamModel: "gpt-5.6"),
        ModelRouteInputV1(logicalModel: "gpt-5.6-sol", upstreamModel: "gpt-5.6-sol"),
    ])
}

@Test func pasteIgnoresBlankLinesAndComments() {
    let routes = ModelRoutePaste.parse("""

        # 从 Sol 控制台复制
        gpt-5.6 = sol-gpt-5.6

        """)
    #expect(routes.count == 1)
}

@Test func firstEntryWinsForDuplicateLogicalModels() {
    // 静默用最后一条覆盖,会让用户以为自己写的那条生效了。
    let routes = ModelRoutePaste.parse("""
        gpt-5.6 = first
        gpt-5.6 = second
        """)
    #expect(routes == [ModelRouteInputV1(logicalModel: "gpt-5.6", upstreamModel: "first")])
}

@Test func pasteStripsQuotesAndStrayPadding() {
    let routes = ModelRoutePaste.parse("""
          "gpt-5.6"  =   "sol-gpt-5.6"
        """)
    #expect(routes == [ModelRouteInputV1(logicalModel: "gpt-5.6", upstreamModel: "sol-gpt-5.6")])
}

@Test func emptyPasteYieldsNothing() {
    #expect(ModelRoutePaste.parse("").isEmpty)
    #expect(ModelRoutePaste.parse("\n\n   \n").isEmpty)
}
#endif
