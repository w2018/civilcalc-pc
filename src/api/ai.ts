/**
 * 组 9：AI 生成（6 个命令 + 4 个流式事件）。
 *
 * 见 docs/08-IPC契约.md §3 组 9 与 §4 事件契约。
 *
 * ## 流式是**命令 + 事件**的组合
 *
 * `invoke` 的 Promise 在**生成结束时**才 resolve（返回值是最终结果）；
 * 生成过程中的正文与思考过程走事件。
 * 所以调用方要**先订阅事件、再发命令**，否则会漏掉开头几段。
 *
 * ## 🔴 载荷是增量
 *
 * `ai://chunk` 与 `ai://thinking` 每次只给这一小段，前端 `+=` 累加。
 * 订阅回调里**不要**用 `=` 赋值。
 *
 * ## 取消
 *
 * `aiCancel()` 会让后端立即断流并让进行中的 `invoke` 以
 * `CommandError.cancelled` 拒绝 —— 调用方要把「用户主动取消」
 * 与「真的失败」区分开（前者不该弹错误提示）。
 */

import { invoke } from './invoke'
import { onTauriEvent } from './events'
import {
  AI_CHUNK_EVENT,
  AI_DONE_EVENT,
  AI_ERROR_EVENT,
  AI_THINKING_EVENT,
  type AiDonePayload,
  type AiTextPayload,
  type NormalizeResult,
  type ResolvedImages,
} from '@/types/ai'
import type { CommandError } from '@/types/error'
import type { FormulaSchema } from '@/types/domain'

/**
 * 解析「公式文本 / 自然语言」为公式。
 *
 * ## ⚠️ 目前**没有 UI 入口**（v1.0.1 合并页面后）
 *
 * 它和 {@link normalizeFromQuery} 在后端是**同一条链路** ——
 * `commands/ai.rs` 里两个命令除了本函数的 `parentImageIds` /
 * `renameTo` / `revisedFromId` 三个续写参数外完全一致。
 *
 * 「新建公式」页统一走 `normalizeFromQuery`；续写场景走 `refineFormula`。
 * 本函数保留是因为它是**契约的一部分**（`docs/08` 组 9 列了这条命令），
 * 且 `parentImageIds` 的「父图排在前」语义是 `refine_formula` 之外
 * 唯一能显式指定父图的入口。将来若做「在既有公式上继续粘贴补充」
 * 之类的功能，直接用这里即可。
 *
 * @param raw 原始文本
 * @param images 本次新增的附图 id（**排在父公式附图之后**）
 * @param parentImageIds **父公式**的附图 id（续写场景，必须排在前）
 * @param renameTo 指定新名称（续写时用 `原名-vN.0`）
 * @param revisedFromId 派生自哪个公式
 */
export function normalizeFromPaste(
  raw: string,
  images: string[] = [],
  parentImageIds: string[] = [],
  renameTo?: string,
  revisedFromId?: string,
): Promise<NormalizeResult> {
  return invoke<NormalizeResult>('normalize_from_paste', {
    raw,
    images,
    parentImageIds,
    renameTo,
    revisedFromId,
  })
}

/** 从自然语言描述生成公式（描述页主链路） */
export function normalizeFromQuery(desc: string, images: string[] = []): Promise<NormalizeResult> {
  return invoke<NormalizeResult>('normalize_from_query', { desc, images })
}

/**
 * 为既有公式按需生成「计算公式详解」。
 *
 * ⚠️ 这条链路**不流式**（源项目亦如此）—— 没有正文事件，
 * 前端等返回值即可；只有 `ai://done` 会发一次用于收尾。
 */
export function explainFormula(schema: FormulaSchema): Promise<FormulaSchema> {
  return invoke<FormulaSchema>('explain_formula', { schema })
}

/** 续写微调：在既有公式基础上按需求改写（名称自动版本化） */
/**
 * 在既有公式基础上按需求改写（微调）。
 *
 * @param schema 被微调的公式
 * @param requirement 微调需求（自然语言）
 * @param images 本次**新增**的附图 id（需求 2）。
 *   父公式自身的附图由后端自动前置，这里只传新加的 —— 顺序铁律：
 *   模型按「第 N 张」写 `{{img:N}}`，父图必须排在前才能与既有详解对齐。
 */
export function refineFormula(
  schema: FormulaSchema,
  requirement: string,
  images: string[] = [],
): Promise<NormalizeResult> {
  return invoke<NormalizeResult>('refine_formula', { schema, requirement, images })
}

/**
 * 把图片 id 列表解析成 `序号 → data URL`（**序号从 1 起**）。
 *
 * 找不到的 id 会**跳过但不占位**，序号保持连续 ——
 * 所以传进去的顺序必须与正文里 `{{img:N}}` 的 N 一致。
 */
export function resolveImages(imageIds: string[]): Promise<ResolvedImages> {
  return invoke<ResolvedImages>('resolve_images', { imageIds })
}

/** 取消当前 AI 任务（立即断流） */
export function aiCancel(): Promise<void> {
  return invoke<void>('ai_cancel')
}

// =============================================================================
// 事件订阅
// =============================================================================

/** 一次 AI 生成的事件回调集合 */
export interface AiStreamHandlers {
  /** 思考过程增量（注意 `+=`） */
  onThinking?: (text: string) => void
  /** 正文增量（注意 `+=`） */
  onChunk?: (text: string) => void
  /** 结束（`ok=false` 时说明这一轮没有产出正文） */
  onDone?: (ok: boolean) => void
  /** 失败 */
  onError?: (e: CommandError) => void
}

/**
 * 订阅一轮 AI 生成的全部事件。
 *
 * 返回 `unlisten`，**务必在组件卸载 / 任务结束时调用**，
 * 否则监听器会累积，下一轮生成会重复累加文本。
 *
 * ⚠️ 必须先 `await` 本函数（拿到 unlisten）**再**发命令，
 * 否则可能漏掉最开始的一段增量。
 */
export async function subscribeAi(handlers: AiStreamHandlers): Promise<() => void> {
  const unlisteners = await Promise.all([
    onTauriEvent<AiTextPayload>(AI_THINKING_EVENT, (p) => handlers.onThinking?.(p.text)),
    onTauriEvent<AiTextPayload>(AI_CHUNK_EVENT, (p) => handlers.onChunk?.(p.text)),
    onTauriEvent<AiDonePayload>(AI_DONE_EVENT, (p) => handlers.onDone?.(p.ok)),
    onTauriEvent<CommandError>(AI_ERROR_EVENT, (p) => handlers.onError?.(p)),
  ])

  return () => {
    for (const un of unlisteners) un()
  }
}
