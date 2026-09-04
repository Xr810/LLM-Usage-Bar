#if NATIVE_PREVIEW_SUPPORT
import Foundation

public enum NativePreviewScenario: String, CaseIterable, Sendable {
    case healthy
    case stale
    case refreshError
    case empty
}

public struct NativePreviewState: Equatable, Sendable {
    public var traySnapshot: TrayUsageSnapshotV1
    public var providerDashboard: ProviderMonitoringDashboardV1
    public var providerActivity: [DashboardTrendBucketV1]
    public var modelDashboard: ModelUsageDashboardV1
    public var agentBreakdown: AgentUsageBreakdownV1
    public var usageEvents: UsageEventPageV1
    public var diagnostics: BridgeRuntimeStatusV1

    public init(
        traySnapshot: TrayUsageSnapshotV1,
        providerDashboard: ProviderMonitoringDashboardV1,
        providerActivity: [DashboardTrendBucketV1],
        modelDashboard: ModelUsageDashboardV1,
        agentBreakdown: AgentUsageBreakdownV1,
        usageEvents: UsageEventPageV1,
        diagnostics: BridgeRuntimeStatusV1
    ) {
        self.traySnapshot = traySnapshot
        self.providerDashboard = providerDashboard
        self.providerActivity = providerActivity
        self.modelDashboard = modelDashboard
        self.agentBreakdown = agentBreakdown
        self.usageEvents = usageEvents
        self.diagnostics = diagnostics
    }
}

public enum NativePreviewFixtures {
    public static let subscriptionProviderId = "preview-codex-work"
    public static let meteredProviderId = "preview-openai-api"
    public static let sampleEventPrefix = "native-preview-event-"

    public static func make(
        scenario: NativePreviewScenario = .healthy,
        now: Date = .now,
        range requestedRange: DashboardRangeV1? = nil
    ) -> NativePreviewState {
        let timestamp = Int64(now.timeIntervalSince1970)
        let range = normalized(
            requestedRange ?? DashboardRangeV1(
                startAt: timestamp - (7 * 24 * 60 * 60),
                endAt: timestamp
            )
        )

        if scenario == .empty {
            return emptyState(now: timestamp, range: range)
        }

        let events = makeEvents(range: range)
        let activityRange = DashboardRangeV1(
            startAt: timestamp - (365 * 24 * 60 * 60),
            endAt: timestamp
        )
        return NativePreviewState(
            traySnapshot: makeTraySnapshot(scenario: scenario, now: timestamp),
            providerDashboard: makeProviderDashboard(scenario: scenario, range: range),
            providerActivity: activityTrend(range: activityRange),
            modelDashboard: makeModelDashboard(range: range),
            agentBreakdown: makeAgentBreakdown(range: range),
            usageEvents: page(
                events.filter { $0.providerId == subscriptionProviderId },
                page: 1,
                pageSize: 50
            ),
            diagnostics: previewDiagnostics
        )
    }

    fileprivate static func makeTraySnapshot(
        scenario: NativePreviewScenario,
        now: Int64
    ) -> TrayUsageSnapshotV1 {
        let stale = scenario == .stale || scenario == .refreshError
        let error = scenario == .refreshError ? "preview_refresh_failed" : nil
        let snapshotStatus: TrayUsageStatus = scenario == .refreshError ? .red : .yellow
        let lastSuccessAt = stale ? now - 3_600 : now - 20
        let recentStart = now - (7 * 24 * 60 * 60)
        let trend = trayTrend(startAt: recentStart, endAt: now)

        let subscriptionProvider = TrayProviderUsageV1(
            providerId: subscriptionProviderId,
            providerName: "Codex Team · Work",
            systemPresetKey: "codex",
            billingKind: .subscription,
            status: .yellow,
            warningReason: "five_hour_pace_high",
            recentUsage: TrayProviderRecentUsageV1(
                startAt: recentStart,
                endAt: now,
                todayTokens: 182_450,
                totalTokens: 1_284_300,
                todayCostUsd: nil,
                totalCostUsd: nil,
                costQuality: .unavailable,
                mostUsedModel: "gpt-5.2-codex",
                trendBuckets: trend
            ),
            subscription: TraySubscriptionUsageV1(
                planLabel: "Team",
                windows: [
                    TrayQuotaWindowV1(
                        kind: "five_hour",
                        usedPercent: "66.5",
                        remainingPercent: "33.5",
                        resetsAt: iso8601(now + 7_200),
                        status: .yellow,
                        unavailableReason: nil,
                        burnRatePercentPerHour: "13.4",
                        projectedExhaustAt: iso8601(now + 8_950),
                        headroomRatio: "0.84",
                        paceBasis: .measured,
                        rhythmAdjustment: "1.08",
                        flatStatus: .yellow
                    ),
                    TrayQuotaWindowV1(
                        kind: "seven_day",
                        usedPercent: nil,
                        remainingPercent: nil,
                        resetsAt: nil,
                        status: .unknown,
                        unavailableReason: "provider_did_not_report_window",
                        burnRatePercentPerHour: nil,
                        projectedExhaustAt: nil,
                        headroomRatio: nil,
                        paceBasis: .static,
                        rhythmAdjustment: nil,
                        flatStatus: nil
                    ),
                ],
                manualResetsRemaining: 2,
                manualResetCredits: [
                    ManualResetCreditV1(
                        id: "preview-credit-1",
                        title: "Monthly reset",
                        expiresAt: iso8601(now + (11 * 24 * 60 * 60))
                    ),
                ]
            ),
            metered: nil
        )

        let meteredProvider = TrayProviderUsageV1(
            providerId: meteredProviderId,
            providerName: "OpenAI API · Personal",
            systemPresetKey: "openai",
            billingKind: .metered,
            status: .green,
            warningReason: nil,
            recentUsage: TrayProviderRecentUsageV1(
                startAt: recentStart,
                endAt: now,
                todayTokens: 48_750,
                totalTokens: 296_200,
                todayCostUsd: "3.42",
                totalCostUsd: "18.73",
                costQuality: .complete,
                mostUsedModel: "gpt-5.1-mini",
                trendBuckets: trend.enumerated().map { index, bucket in
                    var copy = bucket
                    copy.inputTokens = UInt64(6_000 + (index * 750))
                    copy.outputTokens = UInt64(1_400 + (index * 220))
                    copy.totalTokens = copy.inputTokens + copy.outputTokens
                    copy.totalCostUsd = decimalString(0.42 + (Double(index) * 0.07))
                    copy.costSourceCounts = CostSourceCountsV1(
                        upstream: copy.eventCount,
                        estimated: 0,
                        unavailable: 0
                    )
                    return copy
                }
            ),
            subscription: nil,
            metered: TrayMeteredUsageV1(
                todayCostUsd: "3.42",
                rolling30DayCostUsd: "41.87",
                dailyBudgetUsd: "8.00",
                budgetConsumedPercent: "42.75",
                totalTokens: 296_200,
                costQuality: .complete,
                rhythmAdjustment: "0.91",
                flatStatus: .green
            )
        )

        return TrayUsageSnapshotV1(
            status: snapshotStatus,
            generatedAt: now,
            lastSuccessAt: lastSuccessAt,
            stale: stale,
            refreshError: error,
            refreshInProgress: false,
            apiBudget: TrayAPIBudgetV1(
                mode: .shared,
                providerCount: 1,
                todayCostUsd: "3.42",
                dailyBudgetUsd: "8.00",
                budgetConsumedPercent: "42.75",
                costQuality: .complete,
                status: .green,
                warningReason: nil,
                burnRateUsdPerHour: "0.38",
                projectedExhaustAt: iso8601(now + 43_389),
                headroomRatio: "1.34",
                paceBasis: .measured,
                rhythmAdjustment: "0.91",
                flatStatus: .green
            ),
            agents: [
                TrayAgentUsageV1(
                    agentModuleId: "preview-codex-agent",
                    name: "Codex",
                    sortOrder: 0,
                    status: subscriptionProvider.status,
                    providers: [subscriptionProvider]
                ),
                TrayAgentUsageV1(
                    agentModuleId: "preview-automation-agent",
                    name: "Automation",
                    sortOrder: 1,
                    status: meteredProvider.status,
                    providers: [meteredProvider]
                ),
            ]
        )
    }

    fileprivate static func makeProviderDashboard(
        scenario: NativePreviewScenario,
        range: DashboardRangeV1
    ) -> ProviderMonitoringDashboardV1 {
        let fetchError = scenario == .refreshError ? "preview_refresh_failed" : nil
        let stale = scenario == .stale || scenario == .refreshError
        let rows = [
            ProviderDashboardRowV1(
                provider: UsageProviderSummaryV1(
                    id: subscriptionProviderId,
                    name: "Codex Team · Work",
                    billingKind: .subscription,
                    productGroupId: "codex-team",
                    enabled: true,
                    systemPresetKey: "codex"
                ),
                sharedAccount: false,
                eventCount: 55,
                inputTokens: 726_000,
                outputTokens: 182_000,
                cacheReadTokens: 321_000,
                cacheCreationTokens: 55_300,
                totalCostUsd: nil,
                costSourceCounts: CostSourceCountsV1(upstream: 0, estimated: 0, unavailable: 55),
                quota: QuotaStatusV1(
                    snapshotId: "preview-quota-snapshot",
                    fetchedAt: range.endAt - 20,
                    sourceObservedAt: range.endAt - 35,
                    planType: "Team",
                    planRenewsAt: range.endAt + (18 * 24 * 60 * 60),
                    fiveHourUtilizationPercent: "66.5",
                    fiveHourResetsAt: iso8601(range.endAt + 7_200),
                    sevenDayUtilizationPercent: nil,
                    sevenDayResetsAt: nil,
                    fiveHourPace: QuotaWindowPaceV1(
                        status: .yellow,
                        burnRatePercentPerHour: "13.4",
                        projectedExhaustAt: iso8601(range.endAt + 8_950),
                        headroomRatio: "0.84",
                        paceBasis: .measured,
                        rhythmAdjustment: "1.08",
                        flatStatus: .yellow
                    ),
                    sevenDayPace: QuotaWindowPaceV1(
                        status: .unknown,
                        burnRatePercentPerHour: nil,
                        projectedExhaustAt: nil,
                        headroomRatio: nil,
                        paceBasis: .static,
                        rhythmAdjustment: nil,
                        flatStatus: nil
                    ),
                    manualResetsRemaining: 2,
                    manualResetCredits: [
                        ManualResetCreditV1(
                            id: "preview-credit-1",
                            title: "Monthly reset",
                            expiresAt: iso8601(range.endAt + (11 * 24 * 60 * 60))
                        ),
                    ]
                ),
                quotaFetchState: QuotaFetchStateV1(
                    providerId: subscriptionProviderId,
                    lastAttemptAt: range.endAt - 20,
                    lastSuccessAt: stale ? range.endAt - 3_600 : range.endAt - 20,
                    lastError: fetchError,
                    consecutiveFailures: scenario == .refreshError ? 1 : 0,
                    stale: stale
                ),
                tokenSources: [.sessionLog]
            ),
            ProviderDashboardRowV1(
                provider: UsageProviderSummaryV1(
                    id: meteredProviderId,
                    name: "OpenAI API · Personal",
                    billingKind: .metered,
                    productGroupId: "openai-api",
                    enabled: true,
                    systemPresetKey: "openai"
                ),
                sharedAccount: false,
                eventCount: 10,
                inputTokens: 189_000,
                outputTokens: 62_400,
                cacheReadTokens: 38_000,
                cacheCreationTokens: 6_800,
                totalCostUsd: "18.73",
                costSourceCounts: CostSourceCountsV1(upstream: 10, estimated: 0, unavailable: 0),
                quota: nil,
                quotaFetchState: nil,
                tokenSources: [.proxy, .sessionLog]
            ),
        ]

        return ProviderMonitoringDashboardV1(
            startAt: range.startAt,
            endAt: range.endAt,
            providers: rows,
            trendGranularity: granularity(for: range),
            trendBuckets: dashboardTrend(range: range),
            warnings: fetchError.map { [$0] } ?? []
        )
    }

    fileprivate static func makeModelDashboard(range: DashboardRangeV1) -> ModelUsageDashboardV1 {
        let first = range.startAt + min(120, max(0, range.endAt - range.startAt - 1))
        let last = max(first, range.endAt - 300)
        let codexPrimary = ModelUsageRowV1(
            model: "gpt-5.2-codex",
            providerId: subscriptionProviderId,
            providerName: "Codex Team · Work",
            productGroupId: "codex-team",
            billingKind: .subscription,
            eventCount: 37,
            inputTokens: 512_000,
            outputTokens: 142_000,
            cacheReadTokens: 248_000,
            cacheCreationTokens: 41_000,
            totalTokens: 943_000,
            totalCostUsd: nil,
            costSourceCounts: CostSourceCountsV1(upstream: 0, estimated: 0, unavailable: 37),
            firstOccurredAt: first,
            lastOccurredAt: last
        )
        let codexFast = ModelUsageRowV1(
            model: "gpt-5.1-codex-mini",
            providerId: subscriptionProviderId,
            providerName: "Codex Team · Work",
            productGroupId: "codex-team",
            billingKind: .subscription,
            eventCount: 18,
            inputTokens: 214_000,
            outputTokens: 40_000,
            cacheReadTokens: 73_000,
            cacheCreationTokens: 14_300,
            totalTokens: 341_300,
            totalCostUsd: nil,
            costSourceCounts: CostSourceCountsV1(upstream: 0, estimated: 0, unavailable: 18),
            firstOccurredAt: first,
            lastOccurredAt: last
        )
        let metered = ModelUsageRowV1(
            model: "gpt-5.1-mini",
            providerId: meteredProviderId,
            providerName: "OpenAI API · Personal",
            productGroupId: "openai-api",
            billingKind: .metered,
            eventCount: 10,
            inputTokens: 189_000,
            outputTokens: 62_400,
            cacheReadTokens: 38_000,
            cacheCreationTokens: 6_800,
            totalTokens: 296_200,
            totalCostUsd: "18.73",
            costSourceCounts: CostSourceCountsV1(upstream: 10, estimated: 0, unavailable: 0),
            firstOccurredAt: first,
            lastOccurredAt: last
        )
        let all = [codexPrimary, codexFast, metered]

        return ModelUsageDashboardV1(
            startAt: range.startAt,
            endAt: range.endAt,
            totalTokens: all.reduce(0) { $0 + $1.totalTokens },
            totalEventCount: 65,
            totalCostUsd: "18.73",
            productGroups: [
                productGroup(
                    id: "codex-team",
                    billingKind: .subscription,
                    providerIds: [subscriptionProviderId],
                    providerNames: ["Codex Team · Work"],
                    models: [codexPrimary, codexFast]
                ),
                productGroup(
                    id: "openai-api",
                    billingKind: .metered,
                    providerIds: [meteredProviderId],
                    providerNames: ["OpenAI API · Personal"],
                    models: [metered]
                ),
            ],
            models: all.map(modelTotal),
            warnings: []
        )
    }

    fileprivate static func makeAgentBreakdown(range: DashboardRangeV1) -> AgentUsageBreakdownV1 {
        let models = makeModelDashboard(range: range).productGroups.flatMap(\.models)
        let codexModels = Array(models.prefix(2))
        let meteredModels = Array(models.suffix(1))
        let first = range.startAt + min(120, max(0, range.endAt - range.startAt - 1))
        let last = max(first, range.endAt - 300)

        return AgentUsageBreakdownV1(
            startAt: range.startAt,
            endAt: range.endAt,
            totalTokens: 1_580_500,
            totalEventCount: 65,
            totalCostUsd: "18.73",
            agents: [
                AgentUsageRowV1(
                    agentModuleId: "preview-codex-agent",
                    agentName: "Codex",
                    archived: false,
                    visible: true,
                    eventCount: 55,
                    inputTokens: 726_000,
                    outputTokens: 182_000,
                    cacheReadTokens: 321_000,
                    cacheCreationTokens: 55_300,
                    totalTokens: 1_284_300,
                    totalCostUsd: nil,
                    costSourceCounts: CostSourceCountsV1(upstream: 0, estimated: 0, unavailable: 55),
                    firstOccurredAt: first,
                    lastOccurredAt: last,
                    providers: [
                        AgentProviderUsageRowV1(
                            providerId: subscriptionProviderId,
                            providerName: "Codex Team · Work",
                            productGroupId: "codex-team",
                            billingKind: .subscription,
                            eventCount: 55,
                            inputTokens: 726_000,
                            outputTokens: 182_000,
                            cacheReadTokens: 321_000,
                            cacheCreationTokens: 55_300,
                            totalTokens: 1_284_300,
                            totalCostUsd: nil,
                            costSourceCounts: CostSourceCountsV1(upstream: 0, estimated: 0, unavailable: 55)
                        ),
                    ],
                    models: codexModels.map(modelTotal)
                ),
                AgentUsageRowV1(
                    agentModuleId: "preview-automation-agent",
                    agentName: "Automation",
                    archived: false,
                    visible: true,
                    eventCount: 10,
                    inputTokens: 189_000,
                    outputTokens: 62_400,
                    cacheReadTokens: 38_000,
                    cacheCreationTokens: 6_800,
                    totalTokens: 296_200,
                    totalCostUsd: "18.73",
                    costSourceCounts: CostSourceCountsV1(upstream: 10, estimated: 0, unavailable: 0),
                    firstOccurredAt: first,
                    lastOccurredAt: last,
                    providers: [
                        AgentProviderUsageRowV1(
                            providerId: meteredProviderId,
                            providerName: "OpenAI API · Personal",
                            productGroupId: "openai-api",
                            billingKind: .metered,
                            eventCount: 10,
                            inputTokens: 189_000,
                            outputTokens: 62_400,
                            cacheReadTokens: 38_000,
                            cacheCreationTokens: 6_800,
                            totalTokens: 296_200,
                            totalCostUsd: "18.73",
                            costSourceCounts: CostSourceCountsV1(upstream: 10, estimated: 0, unavailable: 0)
                        ),
                    ],
                    models: meteredModels.map(modelTotal)
                ),
            ],
            warnings: []
        )
    }

    fileprivate static func makeEvents(range: DashboardRangeV1) -> [UsageEventV1] {
        let span = max(1, range.endAt - range.startAt)
        return (0..<65).map { index in
            let isSubscription = index < 55
            let providerId = isSubscription ? subscriptionProviderId : meteredProviderId
            let model: String
            if !isSubscription {
                model = "gpt-5.1-mini"
            } else if index.isMultiple(of: 3) {
                model = "gpt-5.1-codex-mini"
            } else {
                model = "gpt-5.2-codex"
            }
            let offset = Int64((index + 1)) * max(1, span / 66)
            let occurredAt = max(range.startAt, range.endAt - offset)
            let input = UInt64(5_000 + (index * 173))
            let output = UInt64(1_100 + (index * 47))
            let cacheRead = UInt64(index.isMultiple(of: 2) ? 2_400 + (index * 31) : 0)
            let cacheCreation = UInt64(index.isMultiple(of: 5) ? 450 + (index * 11) : 0)
            let cost = isSubscription ? nil : decimalString(0.91 + (Double(index - 55) * 0.19))
            return UsageEventV1(
                eventId: "\(sampleEventPrefix)\(String(format: "%03d", index + 1))",
                source: index.isMultiple(of: 4) ? .proxy : .sessionLog,
                providerId: providerId,
                agentModuleId: isSubscription ? "preview-codex-agent" : "preview-automation-agent",
                productGroupId: isSubscription ? "codex-team" : "openai-api",
                occurredAt: occurredAt,
                model: model,
                inputTokens: input,
                outputTokens: output,
                cacheReadTokens: cacheRead,
                cacheCreationTokens: cacheCreation,
                requestId: "preview-request-\(index + 1)",
                sessionId: "preview-session-\((index % 6) + 1)",
                upstreamCorrelationId: nil,
                inputCostUsd: cost,
                outputCostUsd: nil,
                cacheReadCostUsd: nil,
                cacheCreationCostUsd: nil,
                totalCostUsd: cost,
                costSource: isSubscription ? .unavailable : .upstream,
                pricingOrigin: isSubscription ? nil : .official,
                legacyRequestId: nil,
                createdAt: occurredAt
            )
        }
        .sorted { $0.occurredAt > $1.occurredAt }
    }

    fileprivate static func page(
        _ items: [UsageEventV1],
        page: UInt64,
        pageSize: UInt64
    ) -> UsageEventPageV1 {
        let safePage = max(1, page)
        let safePageSize = max(1, pageSize)
        let start = min(UInt64(items.count), (safePage - 1) * safePageSize)
        let end = min(UInt64(items.count), start + safePageSize)
        return UsageEventPageV1(
            items: Array(items[Int(start)..<Int(end)]),
            total: UInt64(items.count),
            page: safePage,
            pageSize: safePageSize
        )
    }

    private static func emptyState(now: Int64, range: DashboardRangeV1) -> NativePreviewState {
        NativePreviewState(
            traySnapshot: TrayUsageSnapshotV1(
                status: .unknown,
                generatedAt: now,
                lastSuccessAt: now,
                stale: false,
                refreshError: nil,
                refreshInProgress: false,
                apiBudget: TrayAPIBudgetV1(
                    mode: .shared,
                    providerCount: 0,
                    todayCostUsd: nil,
                    dailyBudgetUsd: nil,
                    budgetConsumedPercent: nil,
                    costQuality: .unavailable,
                    status: .unknown,
                    warningReason: "no_enabled_metered_providers",
                    burnRateUsdPerHour: nil,
                    projectedExhaustAt: nil,
                    headroomRatio: nil,
                    paceBasis: .idle,
                    rhythmAdjustment: nil,
                    flatStatus: nil
                ),
                agents: []
            ),
            providerDashboard: ProviderMonitoringDashboardV1(
                startAt: range.startAt,
                endAt: range.endAt,
                providers: [],
                trendGranularity: granularity(for: range),
                trendBuckets: [],
                warnings: []
            ),
            providerActivity: [],
            modelDashboard: ModelUsageDashboardV1(
                startAt: range.startAt,
                endAt: range.endAt,
                totalTokens: 0,
                totalEventCount: 0,
                totalCostUsd: nil,
                productGroups: [],
                models: [],
                warnings: []
            ),
            agentBreakdown: AgentUsageBreakdownV1(
                startAt: range.startAt,
                endAt: range.endAt,
                totalTokens: 0,
                totalEventCount: 0,
                totalCostUsd: nil,
                agents: [],
                warnings: []
            ),
            usageEvents: .empty,
            diagnostics: previewDiagnostics
        )
    }

    private static var previewDiagnostics: BridgeRuntimeStatusV1 {
        BridgeRuntimeStatusV1(
            bridgeOnly: false,
            clientCount: 0,
            databaseOwner: "preview-fixture",
            schedulerOwner: "preview-fixture"
        )
    }

    private static func normalized(_ range: DashboardRangeV1) -> DashboardRangeV1 {
        let lowerBound = min(range.startAt, range.endAt)
        let upperBound = max(range.startAt, range.endAt)
        return DashboardRangeV1(
            startAt: lowerBound,
            endAt: upperBound == lowerBound ? lowerBound + 1 : upperBound
        )
    }

    private static func granularity(for range: DashboardRangeV1) -> UsageTrendGranularityV1 {
        range.endAt - range.startAt <= 24 * 60 * 60 ? .hour : .day
    }

    private static func dashboardTrend(range: DashboardRangeV1) -> [DashboardTrendBucketV1] {
        bucketRanges(range: range).enumerated().map { index, bucket in
            let input = UInt64(42_000 + (index * 7_100))
            let output = UInt64(11_000 + (index * 1_900))
            let cacheRead = UInt64(18_000 + (index * 2_400))
            let cacheCreation = UInt64(2_000 + (index * 410))
            return DashboardTrendBucketV1(
                startAt: bucket.startAt,
                endAt: bucket.endAt,
                eventCount: UInt64(4 + (index % 7)),
                inputTokens: input,
                outputTokens: output,
                cacheReadTokens: cacheRead,
                cacheCreationTokens: cacheCreation,
                totalTokens: input + output + cacheRead + cacheCreation,
                totalCostUsd: decimalString(0.78 + (Double(index) * 0.21)),
                costSourceCounts: CostSourceCountsV1(
                    upstream: UInt64(1 + (index % 3)),
                    estimated: UInt64(index % 2),
                    unavailable: UInt64(2 + (index % 4))
                )
            )
        }
    }

    fileprivate static func activityTrend(
        range: DashboardRangeV1
    ) -> [DashboardTrendBucketV1] {
        let range = normalized(range)
        let duration = max(1, range.endAt - range.startAt)
        let dayCount = Int(max(1, min(365, (duration + 86_399) / 86_400)))
        return (0..<dayCount).map { index in
            let start = range.startAt + (duration * Int64(index) / Int64(dayCount))
            let end = index == dayCount - 1
                ? range.endAt
                : range.startAt + (duration * Int64(index + 1) / Int64(dayCount))
            let active = !index.isMultiple(of: 4) && !index.isMultiple(of: 11)
            let intensity = UInt64((index % 5) + 1)
            let input = active ? intensity * 7_400 : 0
            let output = active ? intensity * 1_850 : 0
            let cacheRead = active && index.isMultiple(of: 3) ? intensity * 2_200 : 0
            let cacheCreation = active && index.isMultiple(of: 9) ? intensity * 370 : 0
            let eventCount = active ? UInt64(1 + (index % 6)) : 0
            return DashboardTrendBucketV1(
                startAt: start,
                endAt: max(start + 1, end),
                eventCount: eventCount,
                inputTokens: input,
                outputTokens: output,
                cacheReadTokens: cacheRead,
                cacheCreationTokens: cacheCreation,
                totalTokens: input + output + cacheRead + cacheCreation,
                totalCostUsd: active && index.isMultiple(of: 3)
                    ? decimalString(Double(intensity) * 0.18)
                    : nil,
                costSourceCounts: CostSourceCountsV1(
                    upstream: active && index.isMultiple(of: 3) ? eventCount : 0,
                    estimated: 0,
                    unavailable: active && !index.isMultiple(of: 3) ? eventCount : 0
                )
            )
        }
    }

    private static func trayTrend(startAt: Int64, endAt: Int64) -> [TrayUsageTrendBucketV1] {
        dashboardTrend(range: DashboardRangeV1(startAt: startAt, endAt: endAt)).map {
            TrayUsageTrendBucketV1(
                startAt: $0.startAt,
                endAt: $0.endAt,
                eventCount: $0.eventCount,
                inputTokens: $0.inputTokens,
                outputTokens: $0.outputTokens,
                cacheReadTokens: $0.cacheReadTokens,
                cacheCreationTokens: $0.cacheCreationTokens,
                totalTokens: $0.totalTokens,
                totalCostUsd: nil,
                costSourceCounts: CostSourceCountsV1(
                    upstream: 0,
                    estimated: 0,
                    unavailable: $0.eventCount
                )
            )
        }
    }

    private static func bucketRanges(
        range: DashboardRangeV1
    ) -> [(startAt: Int64, endAt: Int64)] {
        let duration = max(1, range.endAt - range.startAt)
        let targetWidth: Int64 = duration <= 24 * 60 * 60 ? 60 * 60 : 24 * 60 * 60
        let count = Int(max(1, min(30, (duration + targetWidth - 1) / targetWidth)))
        return (0..<count).map { index in
            let start = range.startAt + (duration * Int64(index) / Int64(count))
            let end = index == count - 1
                ? range.endAt
                : range.startAt + (duration * Int64(index + 1) / Int64(count))
            return (start, max(start + 1, end))
        }
    }

    private static func productGroup(
        id: String,
        billingKind: BillingKind,
        providerIds: [String],
        providerNames: [String],
        models: [ModelUsageRowV1]
    ) -> ModelProductGroupV1 {
        ModelProductGroupV1(
            productGroupId: id,
            billingKind: billingKind,
            providerIds: providerIds,
            providerNames: providerNames,
            eventCount: models.reduce(0) { $0 + $1.eventCount },
            inputTokens: models.reduce(0) { $0 + $1.inputTokens },
            outputTokens: models.reduce(0) { $0 + $1.outputTokens },
            cacheReadTokens: models.reduce(0) { $0 + $1.cacheReadTokens },
            cacheCreationTokens: models.reduce(0) { $0 + $1.cacheCreationTokens },
            totalTokens: models.reduce(0) { $0 + $1.totalTokens },
            totalCostUsd: models.compactMap(\.totalCostUsd).first,
            costSourceCounts: models.reduce(CostSourceCountsV1(upstream: 0, estimated: 0, unavailable: 0)) {
                CostSourceCountsV1(
                    upstream: $0.upstream + $1.costSourceCounts.upstream,
                    estimated: $0.estimated + $1.costSourceCounts.estimated,
                    unavailable: $0.unavailable + $1.costSourceCounts.unavailable
                )
            },
            models: models
        )
    }

    private static func modelTotal(_ row: ModelUsageRowV1) -> ModelTotalsRowV1 {
        ModelTotalsRowV1(
            model: row.model,
            providerIds: [row.providerId],
            eventCount: row.eventCount,
            inputTokens: row.inputTokens,
            outputTokens: row.outputTokens,
            cacheReadTokens: row.cacheReadTokens,
            cacheCreationTokens: row.cacheCreationTokens,
            totalTokens: row.totalTokens,
            totalCostUsd: row.totalCostUsd,
            costSourceCounts: row.costSourceCounts,
            firstOccurredAt: row.firstOccurredAt,
            lastOccurredAt: row.lastOccurredAt
        )
    }

    private static func iso8601(_ timestamp: Int64) -> String {
        ISO8601DateFormatter().string(from: Date(timeIntervalSince1970: TimeInterval(timestamp)))
    }

    private static func decimalString(_ value: Double) -> String {
        String(format: "%.2f", locale: Locale(identifier: "en_US_POSIX"), value)
    }
}

public actor PreviewUsageRepository: UsageRepository {
    private let scenario: NativePreviewScenario
    private var timestamp: Int64
    private var snapshotValue: TrayUsageSnapshotV1

    public init(
        scenario: NativePreviewScenario = .healthy,
        now: Date = .now
    ) {
        self.scenario = scenario
        self.timestamp = Int64(now.timeIntervalSince1970)
        self.snapshotValue = NativePreviewFixtures.make(
            scenario: scenario,
            now: now
        ).traySnapshot
    }

    public func snapshot() -> TrayUsageSnapshotV1 { snapshotValue }

    public func refresh() -> TrayUsageSnapshotV1 {
        timestamp += 1
        snapshotValue = NativePreviewFixtures.makeTraySnapshot(
            scenario: scenario,
            now: timestamp
        )
        return snapshotValue
    }

    public func runtimeStatus() -> BridgeRuntimeStatusV1 {
        BridgeRuntimeStatusV1(
            bridgeOnly: false,
            clientCount: 0,
            databaseOwner: "preview-fixture",
            schedulerOwner: "preview-fixture"
        )
    }

    public func shutdownBridge(destination: LegacyHandoffDestinationV1?) async -> Bool { false }
    public func disconnect() async {}
}

public actor PreviewDashboardRepository: DashboardRepository {
    private let scenario: NativePreviewScenario

    public init(scenario: NativePreviewScenario = .healthy) {
        self.scenario = scenario
    }

    public func providerDashboard(
        range: DashboardRangeV1
    ) -> ProviderMonitoringDashboardV1 {
        let range = normalizedRange(range)
        if scenario == .empty {
            return NativePreviewFixtures.make(
                scenario: .empty,
                now: Date(timeIntervalSince1970: TimeInterval(range.endAt)),
                range: range
            ).providerDashboard
        }
        return NativePreviewFixtures.makeProviderDashboard(scenario: scenario, range: range)
    }

    public func modelDashboard(range: DashboardRangeV1) -> ModelUsageDashboardV1 {
        let range = normalizedRange(range)
        if scenario == .empty {
            return NativePreviewFixtures.make(
                scenario: .empty,
                now: Date(timeIntervalSince1970: TimeInterval(range.endAt)),
                range: range
            ).modelDashboard
        }
        return NativePreviewFixtures.makeModelDashboard(range: range)
    }

    public func providerActivity(
        range: DashboardRangeV1
    ) -> [DashboardTrendBucketV1] {
        guard scenario != .empty else { return [] }
        return NativePreviewFixtures.activityTrend(range: normalizedRange(range))
    }

    public func agentBreakdown(range: DashboardRangeV1) -> AgentUsageBreakdownV1 {
        let range = normalizedRange(range)
        if scenario == .empty {
            return NativePreviewFixtures.make(
                scenario: .empty,
                now: Date(timeIntervalSince1970: TimeInterval(range.endAt)),
                range: range
            ).agentBreakdown
        }
        return NativePreviewFixtures.makeAgentBreakdown(range: range)
    }

    public func usageEvents(
        providerId: String,
        range: DashboardRangeV1,
        page: UInt64,
        pageSize: UInt64
    ) -> UsageEventPageV1 {
        guard scenario != .empty else {
            return UsageEventPageV1(items: [], total: 0, page: max(1, page), pageSize: max(1, pageSize))
        }
        let range = normalizedRange(range)
        let events = NativePreviewFixtures.makeEvents(range: range)
            .filter { $0.providerId == providerId }
        return NativePreviewFixtures.page(events, page: page, pageSize: pageSize)
    }

    private nonisolated func normalizedRange(_ range: DashboardRangeV1) -> DashboardRangeV1 {
        let lowerBound = min(range.startAt, range.endAt)
        let upperBound = max(range.startAt, range.endAt)
        return DashboardRangeV1(
            startAt: lowerBound,
            endAt: upperBound == lowerBound ? lowerBound + 1 : upperBound
        )
    }
}
#endif
