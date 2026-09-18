/**
 * LLM 配置类型 —— 与 Rust `civilcalc_llm::{config, error}` 对应。
 *
 * 见 docs/08-IPC契约.md §2.5 与 docs/04-数据契约.md §6。
 */

// =============================================================================
// 枚举
// =============================================================================

/** 思考强度。SCREAMING_SNAKE，与源项目 Kotlin 枚举名一致 */
export type ThinkingLevel = 'AUTO' | 'OFF' | 'LOW' | 'HIGH' | 'MAX'

/** 接入协议。`AUTO` = DeepSeek+联网走 /responses，其余走 /chat/completions */
export type ApiProtocol = 'AUTO' | 'CHAT_COMPLETIONS' | 'RESPONSES'

export const THINKING_LEVEL_LABELS: Record<ThinkingLevel, string> = {
  AUTO: '自动（跟随厂商默认）',
  OFF: '关闭思考',
  LOW: '低',
  HIGH: '高',
  MAX: '最高',
}

export const API_PROTOCOL_LABELS: Record<ApiProtocol, string> = {
  AUTO: '自动',
  CHAT_COMPLETIONS: 'Chat Completions',
  RESPONSES: 'Responses',
}

// =============================================================================
// 档位与配置
// =============================================================================

/**
 * 一个模型档位。
 *
 * 🔒 **结构里没有 API Key 字段** —— Key 只存系统凭据管理器，
 * 只经 `llm_set_api_key` / `llm_save_profile` 写入，永不随本结构返回。
 */
export interface LlmProfile {
  id: string
  label: string
  baseUrl: string
  model: string
  enabled: boolean
  /** 是否联网（web_search） */
  webSearch: boolean
  thinkingLevel: ThinkingLevel
  /** 模型是否支持视觉（图片识别），由用户在设置中判断开启 */
  vision: boolean
  apiProtocol: ApiProtocol
}

export interface LlmConfig {
  profiles: LlmProfile[]
  /** 活跃档位 id */
  active: string
}

/** 三家预设（与后端 `default_profiles()` 一致，仅供 UI 展示用） */
export const DEFAULT_LLM_PROFILES: readonly LlmProfile[] = [
  {
    id: 'deepseek',
    label: 'DeepSeek',
    baseUrl: 'https://api.deepseek.com/v1',
    model: 'deepseek-flash',
    enabled: true,
    webSearch: false,
    thinkingLevel: 'AUTO',
    vision: true,
    apiProtocol: 'AUTO',
  },
  {
    id: 'mimo',
    label: 'MiniMax Mimo',
    baseUrl: 'https://api.xiaomimimo.com/v1',
    model: 'mimo-v2.5',
    enabled: true,
    webSearch: false,
    thinkingLevel: 'AUTO',
    vision: true,
    apiProtocol: 'AUTO',
  },
  {
    id: 'glm',
    label: '智谱 GLM',
    baseUrl: 'https://open.bigmodel.cn/api/paas/v4',
    model: 'glm-5.3-flash',
    enabled: true,
    webSearch: false,
    thinkingLevel: 'AUTO',
    vision: true,
    apiProtocol: 'AUTO',
  },
]

// =============================================================================
// 厂商能力（与后端三个函数一致，用于设置页的提示文案）
// =============================================================================

/**
 * 该 Base URL 对应厂商是否**强制思考**（GLM glm-5.3 系列无法关闭）。
 *
 * ⚠️ 为 `true` 时后端会把 `OFF` **降级为 `LOW`** ——
 * 设置页应在「关闭」选项旁加说明，否则用户会以为没生效。
 */
export function forcesThinking(baseUrl: string): boolean {
  return baseUrl.includes('bigmodel')
}

/** 该厂商在 chat/completions 上是否支持 `reasoning_effort` 档位（小米 MiMo 仅支持开关） */
export function supportsReasoningEffort(baseUrl: string): boolean {
  return !baseUrl.includes('xiaomimimo')
}

/** 该 Base URL 是否支持 `web_search` 联网 */
export function supportsWebSearch(baseUrl: string): boolean {
  return (
    baseUrl.includes('deepseek') ||
    baseUrl.includes('bigmodel') ||
    baseUrl.includes('xiaomimimo')
  )
}

/** 设置页提示：选了「关闭」但厂商强制思考时给用户的说明 */
export function thinkingLevelHint(baseUrl: string, level: ThinkingLevel): string | null {
  if (level === 'OFF' && forcesThinking(baseUrl)) {
    return '该厂商强制思考（发关闭参数会报错），实际会按「低」执行'
  }
  if (
    (level === 'LOW' || level === 'HIGH' || level === 'MAX') &&
    !supportsReasoningEffort(baseUrl)
  ) {
    return '该厂商不支持思考档位，实际会按厂商默认执行'
  }
  return null
}

// =============================================================================
// 测试与导入导出
// =============================================================================

export interface LlmTestResult {
  ok: boolean
  model: string
  latencyMs: number
  message: string
}

export interface LlmConfigImportReport {
  configApplied: boolean
  apiKeysApplied: number
  /** 导入包里有的、本机不存在的档位 id */
  skippedProfiles: string[]
}
