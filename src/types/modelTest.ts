/**
 * 模型测试（组 10）—— 对话式测试的事件与类型。
 *
 * 见 docs/05-项目开发方案.md §1.3.4 与 docs/08-IPC契约.md §3 组 10。
 *
 * ## 事件名与 `ai://*` 同构
 *
 * `modelTest://thinking` / `//content` / `//usage` / `//done` / `//error`。
 * 与 AI 生成的区别：这里**多一个 `//usage`**（每轮实时统计），
 * 且正文事件叫 `content` 而不是 `chunk`。
 *
 * ## 会话是**跨进程持久**的
 *
 * 对话记忆落在偏好键 `model_test_conversation`，
 * 关掉应用再打开还在 —— 由 `modelTestConversation()` 读回。
 */

/**
 * 一轮对话消息。
 *
 * ## 思考过程入库（需求 3）
 *
 * `thinking` 是助手轮的思考全文。**只在助手轮有**，用户轮为 `undefined`。
 *
 * 后端用 `#[serde(default)]` 读它 —— 磁盘上已有的旧对话（没有这个键）
 * 仍能正常读出来，不会因为升级而丢记忆。
 *
 * 发送时前端会把带 `thinking` 的整段对话传回后端：后端**不会**把它拼进
 * API 请求（思考不是对话内容），只是原样保留下来供下次落盘 ——
 * 否则每发一轮，上一轮的思考就会消失一次。
 */
export interface ModelTestTurn {
  /** `"user"` 或 `"assistant"` */
  role: string
  content: string
  /** 思考过程全文（仅助手轮） */
  thinking?: string | null
}

/** `modelTest://thinking` / `//content` 的载荷（**增量**） */
export interface ModelTestTextPayload {
  text: string
}

/** `modelTest://usage` 的载荷（本轮用量） */
export interface ModelTestUsagePayload {
  prompt: number
  completion: number
  total: number
  cached: number
}

/** `modelTest://done` 的载荷 */
export interface ModelTestDonePayload {
  ok: boolean
}

export const MODEL_TEST_THINKING_EVENT = 'modelTest://thinking'
export const MODEL_TEST_CONTENT_EVENT = 'modelTest://content'
export const MODEL_TEST_USAGE_EVENT = 'modelTest://usage'
export const MODEL_TEST_DONE_EVENT = 'modelTest://done'
export const MODEL_TEST_ERROR_EVENT = 'modelTest://error'

/**
 * 默认系统提示词。
 *
 * ⚠️ **与后端 `commands/model_test.rs` 的同名常量逐字一致**，
 * 由契约测试 `default_test_prompt_matches_ts` 钉住（改一边不改另一边会红）。
 *
 * 界面上只在「留空即用默认」的提示语里用到它 —— 真正发给模型的那份
 * 以后端常量为准（前端不参与组装请求）。
 */
export const DEFAULT_TEST_SYSTEM_PROMPT =
  '你是一名专业AI工作助理，具备多领域问题解决能力。\n输出要求：\n1. 回答简洁，逻辑清晰\n2. 优先使用图表呈现关键信息\n3. 回答末尾提供3个相关衍生问题'

/** 对话记忆最多保留的轮数（与后端 `MAX_CONV_TURNS` 一致） */
export const MAX_CONV_TURNS = 200

/**
 * 估算 token（字符数 / 2，与后端同一口径）。
 *
 * 后端在发请求前用它判断是否需要自动压缩；前端用它显示
 * 「当前占用 / 上限」进度条 —— **两边必须同口径**，否则进度条会与
 * 实际触发压缩的时机对不上。
 */
export function estimateTokens(text: string): number {
  return Math.ceil(text.length / 2)
}

/** 整段对话的估算 token（只算正文） */
export function conversationTokens(turns: ModelTestTurn[]): number {
  return turns.reduce((sum, t) => sum + estimateTokens(t.content), 0)
}

/**
 * 是否已接近自动压缩阈值。
 *
 * @param contextSize 上下文窗口大小（偏好 `model_test_context_size`）
 * @param thresholdPercent 压缩阈值百分比（偏好 `model_test_compress_threshold`）
 */
export function nearCompressThreshold(
  turns: ModelTestTurn[],
  contextSize: number,
  thresholdPercent: number,
): boolean {
  if (contextSize <= 0) return false
  const limit = (contextSize * thresholdPercent) / 100
  return conversationTokens(turns) >= limit
}
