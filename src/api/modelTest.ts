/**
 * 组 10：模型测试（5 个命令 + 5 个流式事件）。
 *
 * 见 docs/08-IPC契约.md §3 组 10。
 *
 * ## 与 `api/ai.ts` 的差别
 *
 * - 多一个 `modelTest://usage` 事件（每轮实时统计）
 * - 正文事件叫 `content` 而不是 `chunk`
 * - **没有单独的取消命令** —— 复用 `ai_cancel`
 *   （同一时刻只有一次 LLM 长调用，语义上是「一个活动 AI 交互」）
 *
 * ## 对话落盘时机
 *
 * `modelTestSend` 成功后才把「历史 + 本轮用户 + 助手回复」整体落盘。
 * 所以发送失败时**不要**把用户输入写进本地列表 —— 下次读回会不一致。
 */

import { invoke } from './invoke'
import { onTauriEvent } from './events'
import {
  MODEL_TEST_CONTENT_EVENT,
  MODEL_TEST_DONE_EVENT,
  MODEL_TEST_ERROR_EVENT,
  MODEL_TEST_THINKING_EVENT,
  MODEL_TEST_USAGE_EVENT,
  type ModelTestDonePayload,
  type ModelTestTextPayload,
  type ModelTestTurn,
  type ModelTestUsagePayload,
} from '@/types/modelTest'
import type { CommandError } from '@/types/error'

/**
 * 发送一轮模型测试。
 *
 * @param messages **完整对话**（含本轮新用户消息，排在最后）
 * @param images 贴在本轮用户消息上的 data URL
 */
export function modelTestSend(messages: ModelTestTurn[], images: string[] = []): Promise<void> {
  return invoke<void>('model_test_send', { messages, images })
}

/** 读回持久化的多轮记忆（跨进程保留） */
export function modelTestConversation(): Promise<ModelTestTurn[]> {
  return invoke<ModelTestTurn[]>('model_test_conversation')
}

/** 清空模型测试记忆 */
export function modelTestClear(): Promise<void> {
  return invoke<void>('model_test_clear')
}

/** 设置专属系统提示词（**空串 = 回落默认**） */
export function modelTestSetSystemPrompt(prompt: string): Promise<void> {
  return invoke<void>('model_test_set_system_prompt', { prompt })
}

/**
 * 手动触发上下文压缩：把整段历史压成一条摘要，**替代**原有记忆。
 *
 * ⚠️ 压缩结果会**覆盖**记忆，不是追加。压缩失败时后端不动记忆
 * （但仍会发 `modelTest://done`）。
 */
export function modelTestCompress(): Promise<void> {
  return invoke<void>('model_test_compress')
}

// =============================================================================
// 事件订阅
// =============================================================================

export interface ModelTestStreamHandlers {
  /** 思考过程增量（`+=`） */
  onThinking?: (text: string) => void
  /** 正文增量（`+=`） */
  onContent?: (text: string) => void
  /** 本轮用量 */
  onUsage?: (u: ModelTestUsagePayload) => void
  onDone?: (ok: boolean) => void
  onError?: (e: CommandError) => void
}

/**
 * 订阅一轮模型测试的全部事件。
 *
 * 返回 `unlisten`，务必在组件卸载时调用（否则监听器累积、文本重复累加）。
 * 先 `await` 本函数再 `modelTestSend`。
 */
export async function subscribeModelTest(
  handlers: ModelTestStreamHandlers,
): Promise<() => void> {
  const unlisteners = await Promise.all([
    onTauriEvent<ModelTestTextPayload>(MODEL_TEST_THINKING_EVENT, (p) =>
      handlers.onThinking?.(p.text),
    ),
    onTauriEvent<ModelTestTextPayload>(MODEL_TEST_CONTENT_EVENT, (p) =>
      handlers.onContent?.(p.text),
    ),
    onTauriEvent<ModelTestUsagePayload>(MODEL_TEST_USAGE_EVENT, (p) => handlers.onUsage?.(p)),
    onTauriEvent<ModelTestDonePayload>(MODEL_TEST_DONE_EVENT, (p) => handlers.onDone?.(p.ok)),
    onTauriEvent<CommandError>(MODEL_TEST_ERROR_EVENT, (p) => handlers.onError?.(p)),
  ])

  return () => {
    for (const un of unlisteners) un()
  }
}
