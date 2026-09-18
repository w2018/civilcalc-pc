/**
 * 命令层错误（可判别联合）—— 与 Rust `CommandError` 一一对应。
 *
 * Rust 侧用 `#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]`
 * 序列化，因此 TS 侧可直接按 `kind` 穷尽处理（switch 时 TS 会检查完备性）。
 *
 * ⚠️ `rename_all_fields` 是必须的：`rename_all` 只改**变体名**，
 * 不改结构变体的**字段名**。少了它 `ai_code` 会输出成 `ai_code` 而非 `aiCode`。
 *
 * 见 docs/08-IPC契约.md §1.2 与 docs/04-数据契约.md §7。
 */

/**
 * AI 子错误码（10 个，与源项目 `LlmErrorCode` 枚举名逐字一致）。
 *
 * ⚠️ 后端已把 `aiCode` 映射为**用户可读文案**放在 `message` 里；
 * 这里通常只用于「详情」折叠区或埋点，**不要**自己再翻译一遍。
 */
export type AiErrorCode =
  | 'AUTH_INVALID'
  | 'QUOTA_EXCEEDED'
  | 'MODEL_INVALID'
  | 'NETWORK'
  | 'SCHEMA_PARSE_ERROR'
  | 'OUTPUT_TRUNCATED'
  | 'CONTENT_FILTERED'
  | 'VISION_UNSUPPORTED'
  | 'PROTOCOL_UNSUPPORTED'
  | 'UNKNOWN'

export type CommandError =
  | { kind: 'validation'; message: string }
  | { kind: 'notFound'; message: string }
  | { kind: 'unauthorized'; message: string }
  | { kind: 'network'; message: string }
  | { kind: 'storage'; message: string }
  | { kind: 'aiError'; message: string; aiCode: AiErrorCode }
  | { kind: 'export'; message: string }
  | { kind: 'parse'; message: string }
  | { kind: 'builtinSource'; message: string }
  | { kind: 'pathTraversal'; message: string }
  | { kind: 'invalidArgument'; message: string }
  /**
   * 用户主动取消（上传/下载中途点了「取消」）。
   *
   * 🔴 **静默收尾**：不要弹错误框 —— 用户自己点的取消，再报一次错是噪音。
   * 典型写法：`catch (e) { if ((e as CommandError).kind === 'cancelled') return; ... }`
   */
  | { kind: 'cancelled'; message: string }
  | { kind: 'unknown'; message: string }

/** 错误码 → 中文文案（对齐源项目 9 个 AppError + 10 个 LlmErrorCode） */
export function errorMessage(e: CommandError): string {
  switch (e.kind) {
    case 'validation':
      return e.message
    case 'notFound':
      return e.message || '未找到匹配内容'
    case 'unauthorized':
      return e.message || '未授权，请检查配置'
    case 'network':
      return e.message || '网络连接失败，请检查网络后重试'
    case 'storage':
      return e.message || '存储失败'
    case 'aiError':
      return e.message || 'AI 调用失败，请稍后重试'
    case 'export':
      return e.message || '导出失败'
    case 'parse':
      return e.message || '内容解析失败'
    case 'builtinSource':
      return e.message || '内置公式来源校验失败'
    case 'pathTraversal':
      return e.message || '非法导出路径，已拒绝'
    case 'invalidArgument':
      return e.message
    case 'cancelled':
      return e.message || '已取消'
    case 'unknown':
      return e.message || '操作失败，请稍后重试'
  }
}

/**
 * 该错误是否值得给用户一个「重试」按钮。
 *
 * 见 docs/08-IPC契约.md §1.2 的 kind → UI 形态对照表。
 */
export function isRetryable(e: CommandError): boolean {
  return e.kind === 'network'
}

/**
 * 该错误是否应当引导用户去「设置」页（填 Key / 密码）。
 */
export function isConfigIssue(e: CommandError): boolean {
  return e.kind === 'unauthorized'
}

/**
 * 是否「用户主动取消」——**应当静默收尾**，不弹错误框。
 *
 * 🔴 为什么单独一个 helper：取消在业务上是正常路径，但它在 Promise 层面仍是
 * `reject`。每个 `catch` 都手写 `if (e.kind === 'cancelled') return` 很容易漏，
 * 漏了就变成「用户点了取消，界面弹一个红框说『已取消』」。
 *
 * 典型用法：
 *
 * ```ts
 * try {
 *   await webdavUpload(selection, password)
 * } catch (e) {
 *   if (isCancelled(e as CommandError)) return   // 事件里已经处理过收尾
 *   message.error(errorMessage(e as CommandError))
 * }
 * ```
 */
export function isCancelled(e: CommandError): boolean {
  return e.kind === 'cancelled'
}
