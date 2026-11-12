#if NATIVE_PREVIEW_SUPPORT
import Foundation
import Testing
@testable import UsageCore

private let previewNow = Date(timeIntervalSince1970: 2_000_000_000)
private let previewRange = DashboardRangeV1(
    startAt: 2_000_000_000 - (7 * 24 * 60 * 60),
    endAt: 2_000_000_000
)

@Test func previewScenariosPreserveHealthStaleErrorAndEmptySemantics() {
    let healthy = NativePreviewFixtures.make(scenario: .healthy, now: previewNow)
    let stale = NativePreviewFixtures.make(scenario: .stale, now: previewNow)
    let refreshError = NativePreviewFixtures.make(scenario: .refreshError, now: previewNow)
    let empty = NativePreviewFixtures.make(scenario: .empty, now: previewNow)

    #expect(healthy.traySnapshot.stale == false)
    #expect(healthy.traySnapshot.refreshError == nil)
    #expect(stale.traySnapshot.stale)
    #expect(stale.traySnapshot.refreshError == nil)
    #expect(refreshError.traySnapshot.stale)
    #expect(refreshError.traySnapshot.refreshError == "preview_refresh_failed")
    #expect(empty.traySnapshot.providers.isEmpty)
    #expect(empty.traySnapshot.apiBudget.todayCost == nil)
    #expect(empty.traySnapshot.apiBudget.status == .unknown)
}

@Test func previewKeepsProviderAccountsSeparateAndCoversBillingKinds() {
    let state = NativePreviewFixtures.make(now: previewNow)
    let providers = state.providerDashboard.providers

    #expect(providers.count == 2)
    #expect(Set(providers.map(\.provider.id)).count == 2)
    #expect(providers.contains { $0.provider.billingKind == .subscription })
    #expect(providers.contains { $0.provider.billingKind == .metered })
    #expect(providers.first { $0.provider.billingKind == .subscription }?.totalCost == nil)
    #expect(providers.first { $0.provider.billingKind == .subscription }?.quota?.sevenDayRemainingPercent == nil)
    #expect(state.modelDashboard.models.count == 3)
    #expect(state.agentBreakdown.agents.count == 2)
}

@Test func previewDashboardRebasesBucketsIntoRequestedHalfOpenRange() async {
    let repository = PreviewDashboardRepository()
    let dashboard = await repository.providerDashboard(range: previewRange)

    #expect(dashboard.startAt == previewRange.startAt)
    #expect(dashboard.endAt == previewRange.endAt)
    #expect(dashboard.trendBuckets.count == 7)
    #expect(dashboard.trendBuckets == dashboard.trendBuckets.sorted { $0.startAt < $1.startAt })
    #expect(dashboard.trendBuckets.allSatisfy {
        $0.startAt >= previewRange.startAt
            && $0.startAt < previewRange.endAt
            && $0.endAt > $0.startAt
            && $0.endAt <= previewRange.endAt
    })
}

@Test func previewActivityCovers365LocalDaysAndStaysInsideHalfOpenRange() async {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = TimeZone(identifier: "America/Los_Angeles")!
    let activityRange = DashboardRangeResolver.providerActivity(now: previewNow, calendar: calendar)
    let repository = PreviewDashboardRepository()
    let activity = await repository.providerActivity(range: activityRange)

    #expect(activity.count == 365)
    #expect(activity == activity.sorted { $0.startAt < $1.startAt })
    #expect(activity.contains { $0.totalTokens == 0 })
    #expect(activity.contains { $0.totalTokens > 0 })
    #expect(activity.allSatisfy {
        $0.startAt >= activityRange.startAt
            && $0.startAt < activityRange.endAt
            && $0.endAt > $0.startAt
            && $0.endAt <= activityRange.endAt
    })
}

@Test(arguments: [
    UsageRangePresetV1.today,
    .sevenDays,
    .thirtyDays,
    .oneYear,
])
func presetRangesAreCalendarAlignedAndHalfOpenAcrossDST(_ preset: UsageRangePresetV1) {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = TimeZone(identifier: "America/New_York")!
    let now = ISO8601DateFormatter().date(from: "2026-03-09T16:00:00Z")!
    let range = DashboardRangeResolver.resolve(
        UsageRangeSelectionV1(preset: preset),
        now: now,
        calendar: calendar
    )

    #expect(range.startAt < range.endAt)
    #expect(range.endAt == Int64(now.timeIntervalSince1970) + 1)
    let start = Date(timeIntervalSince1970: TimeInterval(range.startAt))
    #expect(calendar.component(.hour, from: start) == 0)
    #expect(calendar.component(.minute, from: start) == 0)
}

@Test func customRangeNormalizesOrderAndKeepsAOneSecondHalfOpenInterval() {
    let reversed = DashboardRangeResolver.resolve(
        UsageRangeSelectionV1(
            preset: .custom,
            customStartAt: 200,
            customEndAt: 100
        ),
        now: previewNow
    )
    let empty = DashboardRangeResolver.resolve(
        UsageRangeSelectionV1(
            preset: .custom,
            customStartAt: 100,
            customEndAt: 100
        ),
        now: previewNow
    )

    #expect(reversed == DashboardRangeV1(startAt: 100, endAt: 200))
    #expect(empty == DashboardRangeV1(startAt: 100, endAt: 101))
}

@Test func previewEventsFilterByProviderAndPaginateAllSixtyFiveRecords() async {
    let repository = PreviewDashboardRepository()
    let subscriptionPage1 = await repository.usageEvents(
        providerId: NativePreviewFixtures.subscriptionProviderId,
        range: previewRange,
        page: 1,
        pageSize: 50
    )
    let subscriptionPage2 = await repository.usageEvents(
        providerId: NativePreviewFixtures.subscriptionProviderId,
        range: previewRange,
        page: 2,
        pageSize: 50
    )
    let metered = await repository.usageEvents(
        providerId: NativePreviewFixtures.meteredProviderId,
        range: previewRange,
        page: 1,
        pageSize: 50
    )

    #expect(subscriptionPage1.total == 55)
    #expect(subscriptionPage1.items.count == 50)
    #expect(subscriptionPage2.items.count == 5)
    #expect(metered.total == 10)
    #expect(subscriptionPage1.total + metered.total == 65)
    #expect(subscriptionPage1.items.allSatisfy {
        $0.providerId == NativePreviewFixtures.subscriptionProviderId
            && $0.occurredAt >= previewRange.startAt
            && $0.occurredAt < previewRange.endAt
    })
    #expect(metered.items.allSatisfy { $0.providerId == NativePreviewFixtures.meteredProviderId })
}

@Test func previewRefreshOnlyAdvancesInMemorySnapshot() async {
    let repository = PreviewUsageRepository(now: previewNow)
    let before = await repository.snapshot()
    let refreshed = await repository.refresh()
    let after = await repository.snapshot()
    let diagnostics = await repository.runtimeStatus()

    #expect(refreshed.generatedAt == before.generatedAt + 1)
    #expect(after == refreshed)
    #expect(after.providers.map(\.providerId) == before.providers.map(\.providerId))
    #expect(diagnostics.databaseOwner == "preview-fixture")
    #expect(diagnostics.schedulerOwner == "preview-fixture")
}
#endif
