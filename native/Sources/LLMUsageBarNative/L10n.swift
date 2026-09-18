#if os(macOS)
import Foundation
import UsageCore

enum L10n {
    enum Key {
        case appName, refresh, details, settings, quit, stale, lastUpdated
        case unavailable, bridgeUnavailable, refreshFailed, budget, todayCost
        case dailyBudget, burnRate, projectedExhaustion, reset, tokens, model
        case providers, models, agents, overview, legacySettings, readOnlyNotice
        case rolling30Days, totalCost, consumed, pace, quality, headroom, resetCredits
        case timeRange, last24Hours, last7Days, last30Days, events, requests, noData
        case inputTokens, outputTokens, sharedAccount, subscription, metered, previous, next
        case providerMonitoring, providerMonitoringDescription, byModel, byModelDescription
        case byAgent, byAgentDescription, dailyActivity, activityRange, activeDays
        case activityUnavailable, usageTrend, hourly, daily, remainingQuota, meteredAccounts
        case meteredOverview, recentRequests, viewAll, calls, sourceProxy, sourceSession
        case today, sevenDays, thirtyDays, oneYear, customRange, apply, cancel, liveEnd
        case startDate, endDate, fiveHourWindow, weeklyAllowance, remaining, resetTimeUnknown
        case noProviders, noSubscriptionProviders, noMeteredProviders, costUnavailable
        case byPlan, modelBreakdown, agentBreakdown, share, accounts, archived, unassigned
        case general, diagnostics, settingsDescription, previewReadOnly, currentAppSettings
        case databaseOwner, schedulerOwner, clients, connection, connected, disconnected
        case configuration, status, accountCount, loading, peak, plan, latestActivity
        #if NATIVE_PREVIEW
        case sampleData
        #endif
    }

    static func text(_ key: Key, detail: String? = nil) -> String {
        #if NATIVE_PREVIEW
        if key == .sampleData {
            return ["Sample Data", "样例数据", "樣例資料", "サンプルデータ"][languageIndex]
        }
        #endif
        let values: [Key: [String]] = [
            .appName: ["LLM Usage Bar", "LLM Usage Bar", "LLM Usage Bar", "LLM Usage Bar"],
            .refresh: ["Refresh", "刷新", "重新整理", "更新"],
            .details: ["Details", "详情", "詳細資料", "詳細"],
            .settings: ["Settings", "设置", "設定", "設定"],
            .quit: ["Quit", "退出", "結束", "終了"],
            .stale: ["Last known data", "上次成功数据", "上次成功資料", "前回成功時のデータ"],
            .lastUpdated: ["Updated", "更新时间", "更新時間", "更新"],
            .unavailable: ["Unavailable", "不可用", "無法使用", "利用不可"],
            .bridgeUnavailable: ["Rust bridge and cached snapshot are unavailable.", "Rust 桥接和缓存快照均不可用。", "Rust 橋接與快取快照均無法使用。", "Rust ブリッジとキャッシュを利用できません。"],
            .refreshFailed: ["Refresh failed: \(detail ?? "unknown")", "刷新失败：\(detail ?? "未知")", "重新整理失敗：\(detail ?? "未知")", "更新失敗：\(detail ?? "不明")"],
            .budget: ["API budget", "API 预算", "API 預算", "API 予算"],
            .todayCost: ["Today", "今日花费", "今日花費", "本日の費用"],
            .dailyBudget: ["Daily budget", "每日预算", "每日預算", "日次予算"],
            .burnRate: ["Burn rate", "消耗速度", "消耗速度", "消費速度"],
            .projectedExhaustion: ["Projected exhaustion", "预计耗尽", "預計用盡", "予測枯渇"],
            .reset: ["Resets", "重置", "重設", "リセット"],
            .tokens: ["tokens", "tokens", "tokens", "tokens"],
            .model: ["Model", "模型", "模型", "モデル"],
            .providers: ["Providers", "Providers", "Providers", "Providers"],
            .models: ["Models", "Models", "Models", "Models"],
            .agents: ["Agents", "Agents", "Agents", "Agents"],
            .overview: ["Overview", "概览", "概覽", "概要"],
            .legacySettings: ["Continue in the current app", "前往当前正式版", "前往目前正式版", "現在の正式版を開く"],
            .readOnlyNotice: ["This preview is read-only. Settings remain in the current app.", "此预览版只读，设置仍由当前正式版管理。", "此預覽版為唯讀，設定仍由目前正式版管理。", "このプレビューは読み取り専用です。設定は現在の正式版で管理します。"],
            .rolling30Days: ["Last 30 days", "近 30 天", "近 30 天", "過去 30 日"],
            .totalCost: ["Total cost", "总花费", "總花費", "合計費用"],
            .consumed: ["Consumed", "已使用", "已使用", "使用済み"],
            .pace: ["Pace", "消耗节奏", "消耗節奏", "消費ペース"],
            .quality: ["Cost quality", "花费质量", "花費品質", "費用品質"],
            .headroom: ["Headroom", "余量", "餘量", "余裕"],
            .resetCredits: ["Reset credits", "重置额度", "重設額度", "リセット枠"],
            .timeRange: ["Time range", "时间范围", "時間範圍", "期間"],
            .last24Hours: ["24 hours", "24 小时", "24 小時", "24 時間"],
            .last7Days: ["7 days", "7 天", "7 天", "7 日"],
            .last30Days: ["30 days", "30 天", "30 天", "30 日"],
            .events: ["Activity", "活动记录", "活動記錄", "アクティビティ"],
            .requests: ["records", "条记录", "筆記錄", "件"],
            .noData: ["No usage in this range", "此时间范围内没有用量", "此時間範圍內沒有用量", "この期間の使用量はありません"],
            .inputTokens: ["Input", "输入", "輸入", "入力"],
            .outputTokens: ["Output", "输出", "輸出", "出力"],
            .sharedAccount: ["Shared account", "共享账号", "共用帳號", "共有アカウント"],
            .subscription: ["Subscription", "订阅", "訂閱", "サブスクリプション"],
            .metered: ["Metered", "按量计费", "按量計費", "従量課金"],
            .previous: ["Previous", "上一页", "上一頁", "前へ"],
            .next: ["Next", "下一页", "下一頁", "次へ"]
            ,.providerMonitoring: ["Provider monitoring", "Provider 监控", "Provider 監控", "Provider モニタリング"]
            ,.providerMonitoringDescription: ["Usage, cost and remaining quota per Provider account.", "按 Provider 账号查看用量、花费和剩余额度。", "依 Provider 帳號查看用量、花費與剩餘額度。", "Provider アカウント別の使用量、費用、残り枠。"]
            ,.byModel: ["By model", "按模型", "依模型", "モデル別"]
            ,.byModelDescription: ["Roll usage up by plan or compare models across accounts.", "按订阅计划汇总，或跨账号比较模型用量。", "依訂閱方案彙總，或跨帳號比較模型用量。", "プラン別集計またはアカウント横断のモデル比較。"]
            ,.byAgent: ["By agent", "按 Agent", "依 Agent", "Agent 別"]
            ,.byAgentDescription: ["See what each Agent used without changing its routing.", "查看每个 Agent 的用量，不改变其路由。", "查看每個 Agent 的用量，不變更其路由。", "ルーティングを変更せず Agent ごとの使用量を表示。"]
            ,.dailyActivity: ["Daily activity", "每日活动", "每日活動", "日別アクティビティ"]
            ,.activityRange: ["Last 12 months · local calendar days", "最近 12 个月 · 本地日历日", "最近 12 個月 · 本地日曆日", "過去 12 か月 · ローカル暦日"]
            ,.activeDays: ["active days", "个活跃日", "個活躍日", "アクティブ日"]
            ,.activityUnavailable: ["Activity is unavailable from this bridge.", "当前 Bridge 无法提供 Activity 数据。", "目前 Bridge 無法提供 Activity 資料。", "この Bridge では Activity を利用できません。"]
            ,.usageTrend: ["Usage trend", "用量趋势", "用量趨勢", "利用トレンド"]
            ,.hourly: ["Hourly", "每小时", "每小時", "時間別"]
            ,.daily: ["Daily", "每日", "每日", "日別"]
            ,.remainingQuota: ["Remaining quota", "剩余额度", "剩餘額度", "残りクォータ"]
            ,.meteredAccounts: ["Metered accounts", "按量计费账号", "按量計費帳號", "従量課金アカウント"]
            ,.meteredOverview: ["Metered overview", "按量计费概览", "按量計費概覽", "従量課金の概要"]
            ,.recentRequests: ["Recent requests", "最近请求", "最近請求", "最近のリクエスト"]
            ,.viewAll: ["View all", "查看全部", "查看全部", "すべて表示"]
            ,.calls: ["Calls", "调用", "呼叫", "呼び出し"]
            ,.sourceProxy: ["Proxy", "代理记录", "代理記錄", "プロキシ"]
            ,.sourceSession: ["Session log", "会话日志", "工作階段日誌", "セッションログ"]
            ,.today: ["Today", "当天", "當天", "今日"]
            ,.sevenDays: ["7 days", "7 天", "7 天", "7 日"]
            ,.thirtyDays: ["30 days", "30 天", "30 天", "30 日"]
            ,.oneYear: ["1 year", "1 年", "1 年", "1 年"]
            ,.customRange: ["Custom range", "自定义范围", "自訂範圍", "カスタム期間"]
            ,.apply: ["Apply", "应用", "套用", "適用"]
            ,.cancel: ["Cancel", "取消", "取消", "キャンセル"]
            ,.liveEnd: ["Keep end time live", "结束时间跟随当前时间", "結束時間跟隨目前時間", "終了時刻を現在時刻に追従"]
            ,.startDate: ["Start", "开始", "開始", "開始"]
            ,.endDate: ["End", "结束", "結束", "終了"]
            ,.fiveHourWindow: ["5-hour window", "5 小时窗口", "5 小時視窗", "5 時間ウィンドウ"]
            ,.weeklyAllowance: ["Weekly allowance", "每周额度", "每週額度", "週間枠"]
            ,.remaining: ["left", "剩余", "剩餘", "残り"]
            ,.resetTimeUnknown: ["Reset time not reported", "未提供重置时间", "未提供重設時間", "リセット時刻未報告"]
            ,.noProviders: ["No Provider accounts are configured.", "尚未配置 Provider 账号。", "尚未設定 Provider 帳號。", "Provider アカウントが設定されていません。"]
            ,.noSubscriptionProviders: ["No subscription Provider accounts.", "没有订阅 Provider 账号。", "沒有訂閱 Provider 帳號。", "サブスクリプション Provider はありません。"]
            ,.noMeteredProviders: ["No metered Provider accounts.", "没有按量计费 Provider 账号。", "沒有按量計費 Provider 帳號。", "従量課金 Provider はありません。"]
            ,.costUnavailable: ["Cost unavailable", "花费不可用", "花費無法使用", "費用を利用できません"]
            ,.byPlan: ["By plan", "按计划", "依方案", "プラン別"]
            ,.modelBreakdown: ["Model breakdown", "模型明细", "模型明細", "モデル内訳"]
            ,.agentBreakdown: ["Agent breakdown", "Agent 明细", "Agent 明細", "Agent 内訳"]
            ,.share: ["Share", "占比", "佔比", "比率"]
            ,.accounts: ["Accounts", "账号", "帳號", "アカウント"]
            ,.archived: ["Archived", "已归档", "已封存", "アーカイブ済み"]
            ,.unassigned: ["Unassigned", "未分配", "未指派", "未割り当て"]
            ,.general: ["General", "通用", "一般", "一般"]
            ,.diagnostics: ["Diagnostics", "诊断", "診斷", "診断"]
            ,.settingsDescription: ["Read-only native preview and runtime status.", "只读原生预览与运行状态。", "唯讀原生預覽與執行狀態。", "読み取り専用ネイティブプレビューと実行状態。"]
            ,.previewReadOnly: ["This native preview is read-only. The current app still owns settings and credentials.", "此原生预览为只读，设置和凭据仍由当前正式版管理。", "此原生預覽為唯讀，設定與憑證仍由目前正式版管理。", "このネイティブプレビューは読み取り専用です。設定と認証情報は現行版が管理します。"]
            ,.currentAppSettings: ["Continue in the current app", "前往当前正式版设置", "前往目前正式版設定", "現行版の設定を開く"]
            ,.databaseOwner: ["Database owner", "数据库所有者", "資料庫擁有者", "データベース所有者"]
            ,.schedulerOwner: ["Scheduler owner", "调度器所有者", "排程器擁有者", "スケジューラ所有者"]
            ,.clients: ["Clients", "客户端", "用戶端", "クライアント"]
            ,.connection: ["Connection", "连接", "連線", "接続"]
            ,.connected: ["Connected", "已连接", "已連線", "接続済み"]
            ,.disconnected: ["Unavailable", "不可用", "無法使用", "利用不可"]
            ,.configuration: ["Configuration", "配置", "設定", "構成"]
            ,.status: ["Status", "状态", "狀態", "状態"]
            ,.accountCount: ["accounts", "个账号", "個帳號", "アカウント"]
            ,.loading: ["Loading…", "正在加载…", "正在載入…", "読み込み中…"]
            ,.peak: ["Peak", "峰值", "峰值", "ピーク"]
            ,.plan: ["Plan", "计划", "方案", "プラン"]
            ,.latestActivity: ["Latest activity", "最近活动", "最近活動", "最新アクティビティ"]
        ]
        return values[key]?[languageIndex] ?? ""
    }

    static func status(_ status: TrayUsageStatus) -> String {
        let values: [[String]] = [
            ["Healthy", "正常", "正常", "正常"],
            ["Warning", "警告", "警告", "警告"],
            ["Critical", "严重", "嚴重", "重大"],
            ["Unavailable", "不可用", "無法使用", "利用不可"]
        ]
        let index = TrayUsageStatus.allCases.firstIndex(of: status) ?? 3
        return values[index][languageIndex]
    }

    private static var languageIndex: Int {
        let identifier = Locale.preferredLanguages.first?.lowercased() ?? "en"
        if identifier.hasPrefix("zh-hant") || identifier.hasPrefix("zh-tw") || identifier.hasPrefix("zh-hk") { return 2 }
        if identifier.hasPrefix("zh") { return 1 }
        if identifier.hasPrefix("ja") { return 3 }
        return 0
    }
}
#endif
