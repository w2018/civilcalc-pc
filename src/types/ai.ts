/**
 * AI 生成（组 9）—— 事件与结果类型。
 *
 * 见 docs/08-IPC契约.md §3 组 9 与 §4 事件契约。
 *
 * ## 🔴 事件载荷是**增量**，不是累积全文
 *
 * `ai://chunk` / `ai://thinking` 每次只给这一小段，前端必须 `+=`。
 * 源项目给的是累积全文，这里改成增量是为了省 IPC 带宽
 * （后端 `AiEmitter` 做差分 + ≥50ms 节流）。
 *
 * ## 🔴 事件名大小写敏感，且**没有前缀通配**
 *
 * 四个事件要逐个订阅。名字是 `ai://` 而**不是** `llm://`。
 */

import type { FormulaSchema } from './domain'

// =============================================================================
// 事件
// =============================================================================

/** AI 正文增量 */
export const AI_CHUNK_EVENT = 'ai://chunk'
/** AI 思考过程增量 */
export const AI_THINKING_EVENT = 'ai://thinking'
/** 生成结束（无论成功失败都会发，用于收尾） */
export const AI_DONE_EVENT = 'ai://done'
/** 生成失败（载荷是 `CommandError`） */
export const AI_ERROR_EVENT = 'ai://error'

/** 四个 AI 事件名（订阅时逐个 `listen`） */
export const AI_EVENTS = [
  AI_CHUNK_EVENT,
  AI_THINKING_EVENT,
  AI_DONE_EVENT,
  AI_ERROR_EVENT,
] as const

/** `ai://chunk` / `ai://thinking` 的载荷 */
export interface AiTextPayload {
  text: string
}

/** `ai://done` 的载荷 */
export interface AiDonePayload {
  ok: boolean
}

// =============================================================================
// 结果
// =============================================================================

/**
 * 归一化用量（与 Rust `civilcalc_llm::NormalizeUsage` 同形）。
 *
 * ⚠️ 厂商未返回用量时整个 `usage` 为 `null` —— **后端不臆造**，
 * 前端也不要补 0，否则「用量页显示 0」与「厂商没给」会分不清。
 */
export interface NormalizeUsage {
  promptTokens: number
  completionTokens: number
  totalTokens: number
  cachedTokens: number
  reasoningTokens: number
}

/** 一次 AI 生成的结果（`normalize_*` / `refine_formula` 的返回） */
export interface NormalizeResult {
  schema: FormulaSchema
  /** 厂商未返回用量时为 `null` */
  usage: NormalizeUsage | null
}

/**
 * `resolve_images` 的返回：`序号 → data URL`（**序号从 1 起**）。
 *
 * ⚠️ Rust 侧是 `BTreeMap<usize, String>`，JSON 的键**必然是字符串**
 * （`{"1": "data:image/jpeg;base64,..."}`），所以这里用 `Record<string, string>`。
 * 找不到的 ID 会被**跳过但不占位**，序号保持连续。
 */
export type ResolvedImages = Record<string, string>

// =============================================================================
// 辅助
// =============================================================================

/**
 * 把 `resolve_images` 的返回按序号取图（序号从 1 起）。
 *
 * 缺失时返回 `null` —— 调用方据此渲染「图片已删除」占位。
 */
export function imageAt(images: ResolvedImages, index: number): string | null {
  return images[String(index)] ?? null
}

/**
 * 从正文里抽出全部 `{{img:N}}` 的序号（去重 + 升序）。
 *
 * 用于「按需解析图片」：只有正文真的引用了才去取 data URL，
 * 避免为一张没被引用的图白跑一次 IPC。
 */
export function referencedImageIndexes(text: string): number[] {
  const out = new Set<number>()
  const re = /\{\{img:(\d+)\}\}/g
  let m: RegExpExecArray | null
  while ((m = re.exec(text)) !== null) {
    const n = Number(m[1])
    if (Number.isFinite(n) && n > 0) out.add(n)
  }
  return [...out].sort((a, b) => a - b)
}

/**
 * 把正文按 `{{img:N}}` 切成「文本段 / 图片段」。
 *
 * 返回的 `kind: 'image'` 段里 `index` 是 `{{img:N}}` 的 N。
 * 这样渲染时可以把图片**内联**在正确位置，而不是统一堆到文末。
 */
export type ExplanationPart =
  | { kind: 'text'; text: string }
  | { kind: 'image'; index: number }

export function splitByImageRefs(text: string): ExplanationPart[] {
  const parts: ExplanationPart[] = []
  const re = /\{\{img:(\d+)\}\}/g
  let last = 0
  let m: RegExpExecArray | null
  while ((m = re.exec(text)) !== null) {
    if (m.index > last) parts.push({ kind: 'text', text: text.slice(last, m.index) })
    const n = Number(m[1])
    if (Number.isFinite(n) && n > 0) {
      parts.push({ kind: 'image', index: n })
    } else {
      // 序号非法 → 原样当文本，不要吞掉用户内容
      parts.push({ kind: 'text', text: m[0] })
    }
    last = m.index + m[0].length
  }
  if (last < text.length) parts.push({ kind: 'text', text: text.slice(last) })
  return parts
}
