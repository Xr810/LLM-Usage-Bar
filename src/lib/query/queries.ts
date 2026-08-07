import { useQuery, type UseQueryResult } from "@tanstack/react-query";
import { settingsApi } from "@/lib/api";
import type { Settings, UsageResult } from "@/types";

export const useSettingsQuery = (): UseQueryResult<Settings> => {
  return useQuery({
    queryKey: ["settings"],
    queryFn: async () => settingsApi.get(),
  });
};

export interface UseUsageQueryOptions {
  enabled?: boolean;
  autoQueryInterval?: number; // 自动查询间隔（分钟），0 表示禁用
}

/** keep-last-good 判定所需的最小结果形状（UsageResult / SubscriptionQuota 都满足）。 */
export interface UsageLikeResult {
  success: boolean;
  error?: string | null;
}

/** 最近一次成功的结果快照（keep-last-good 用）。 */
export interface LastGoodSnapshot<T> {
  data: T;
  at: number; // 该成功结果的获取时刻（ms）
}

/** 脚本路径 UsageResult 的快照类型（历史别名）。 */
export type LastGoodUsage = LastGoodSnapshot<UsageResult>;

/** 在最近一次成功后多久内，失败仍继续展示该成功值。 */
export const KEEP_LAST_GOOD_MS = 10 * 60 * 1000; // 10 分钟

/**
 * 判断一次用量/额度查询失败是否属于"瞬时"（可被 keep-last-good 短暂掩盖）。
 *
 * 仅瞬时失败才允许继续展示上一次成功；**确定性失败**（鉴权失败、空 API Key、
 * 未知供应商、4xx、脚本/解析错误等）必须立即透出——用户改/删凭据后要马上看到，
 * 否则会一直显示过期额度直到窗口结束。
 *
 * 注：后端已把纯传输层失败（send 失败/超时/读体中断）转成 Err → invoke reject，
 * 不再折叠进 `Ok(success:false)`，到不了这里——那类失败由 [`resolveDisplayUsage`]
 * 的 `rejected` 分支按同一窗口处理。本白名单主要兜仍以 `success:false` 返回的
 * **HTTP 5xx/429**，网络类文案匹配仅作兼容冗余。
 *
 * 采用**白名单**，失败安全——任何未识别的错误一律按"非瞬时"立即透出，绝不误掩盖
 * 确定性失败。需与后端错误文案保持同步：
 * - 原生 balance/coding_plan/subscription：上游非 2xx → `"API error (HTTP <code>…)"`
 * - JS 脚本 usage_script：上游非 2xx → `"HTTP <code> …"`
 *
 * HTTP 状态：**5xx**（服务端错误，通常瞬时）与 **429**（限流，稍后重试即可）归为
 * transient；其余 **4xx**（鉴权/客户端错误，如 401/403/404）保持确定性，立即透出。
 */
export function isTransientUsageError(result: UsageLikeResult): boolean {
  if (result.success) return false;
  const e = result.error?.toLowerCase() ?? "";
  if (!e) return false;

  // 网络类（send 失败/超时/读取响应失败）
  if (
    e.includes("network error") || // 原生路径
    e.includes("request failed") || // JS 脚本 (en)
    e.includes("请求失败") || // JS 脚本 (zh)
    e.includes("failed to read response") || // JS 脚本 (en)
    e.includes("读取响应失败") // JS 脚本 (zh)
  ) {
    return true;
  }

  // HTTP 状态码：5xx 与 429（限流）视为瞬时，其余 4xx 视为确定性。错误文案里第一处
  // "HTTP <code>" 即为上游状态码（原生 "API error (HTTP 500…)"、JS 脚本 "HTTP 500 …"）。
  const httpMatch = e.match(/http\s+(\d{3})/);
  if (httpMatch) {
    const status = Number(httpMatch[1]);
    return (status >= 500 && status <= 599) || status === 429;
  }

  return false;
}

/**
 * Keep-last-good 的纯决策函数（无 ref、无时钟，`now` 注入以便测试）。
 * 对 `UsageResult`（脚本路径）与 `SubscriptionQuota`（订阅系 hooks）通用。
 *
 * 策略：当失败是**瞬时**（见 [`isTransientUsageError`]，即 5xx/429 等）且最近一次
 * 成功在 `keepMs` 内时，不抹掉它，继续展示该成功值，并把 `lastQueriedAt` 指向该
 * 成功的时刻（相对时间自然走到"10 分钟前"后过期翻红）；超出窗口、或从无成功记录
 * 时照常展示失败。
 *
 * **确定性失败**（鉴权/空 key/未知供应商/4xx 等）不仅立即透出，还会**清空 `lastGood`**：
 * 旧成功快照已不可信，否则随后一次网络抖动会把"配置/鉴权已失效"的旧额度重新复活。
 *
 * 会 reject 的传输层失败（网络/超时/读体中断，后端已转 Err）由 `rejected` 标志
 * 处理：react-query 保留的上次成功 `data` 是**陈旧**的，不能当新鲜成功无限期展示
 * ——同样只在 `keepMs` 窗口内（锚定上次真实成功时刻）继续展示，超窗后 `data`
 * 置空，由调用方合成失败占位透出。否则「彻底断网」反而比「单次 5xx」掩盖更久。
 */
export interface ResolveDisplayUsageOptions {
  /** 本次查询是否以 reject 告终（invoke Err；react-query 会保留上次成功 data）。 */
  rejected?: boolean;
  keepMs?: number;
}

export function resolveDisplayUsage<T extends UsageLikeResult>(
  raw: T | undefined,
  dataUpdatedAt: number,
  prevLastGood: LastGoodSnapshot<T> | null,
  now: number,
  options: ResolveDisplayUsageOptions = {},
): {
  data: T | undefined;
  lastQueriedAt: number | null;
  lastGood: LastGoodSnapshot<T> | null;
} {
  const { rejected = false, keepMs = KEEP_LAST_GOOD_MS } = options;

  if (rejected && raw?.success) {
    // reject 时看到的"成功"是 react-query 保留的旧值。用它补种 lastGood——锚定
    // 上次真实成功时刻（dataUpdatedAt），且从 query 缓存派生，组件重挂丢失 ref
    // 后窗口判定依然成立。
    const lastGood = { data: raw, at: dataUpdatedAt || now };
    if (now - lastGood.at < keepMs) {
      return { data: raw, lastQueriedAt: lastGood.at, lastGood };
    }
    // 超窗：陈旧成功不再展示（调用方合成失败占位）。快照保留而非清空——reject
    // 属瞬时、不代表旧值失信（与 5xx 的"不清空"一致）；窗口锚定成功时刻，
    // 超窗后自然保持失效，直到下次成功刷新。
    return { data: undefined, lastQueriedAt: lastGood.at, lastGood };
  }

  let lastGood = prevLastGood;
  if (raw?.success) {
    // 成功：刷新快照
    lastGood = { data: raw, at: dataUpdatedAt || now };
  } else if (raw && !isTransientUsageError(raw)) {
    // 确定性失败（鉴权/空 key/未知供应商/4xx 等）：旧成功快照已不可信，丢弃它，
    // 避免后续一次网络抖动把"配置/鉴权已失效"的旧额度重新复活。
    lastGood = null;
  }

  let data = raw;
  let lastQueriedAt = dataUpdatedAt || null;
  if (
    raw &&
    !raw.success &&
    isTransientUsageError(raw) && // 仅瞬时/网络类失败才掩盖；确定性失败立即透出
    lastGood &&
    now - lastGood.at < keepMs
  ) {
    data = lastGood.data;
    lastQueriedAt = lastGood.at;
  }

  return { data, lastQueriedAt, lastGood };
}
