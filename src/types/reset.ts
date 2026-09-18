/**
 * 重置软件（组 14）—— 9 类可重置项 + 真实存量 + 二次确认。
 *
 * 见 docs/08-IPC契约.md §3 组 14，与 Rust `civilcalc_core::reset` 一一对应。
 *
 * ## ⚠️ 一行文案逻辑在这里被**镜像**了一份
 *
 * `subtitleOf()` 是 Rust `ResetCounts::subtitle_of` 的 TS 镜像。
 * 契约只给了 3 个命令（`reset_counts` / `reset_confirm_lines` / `reset_execute`），
 * 没有「逐行副标题」的命令，所以只能在前端拼。
 * **改一边必须改另一边** —— Rust 侧有单测（`subtitle_text`）钉住格式。
 *
 * 「二次确认」的正文**不走镜像**：用 `reset_confirm_lines` 命令拿，
 * 保证弹窗里写的和实际做的来自同一份代码。
 */

// =============================================================================
// 类别
// =============================================================================

/** 9 个可重置类别（字符串值与 `ResetSelection` 的字段名一致） */
export type ResetSectionKey =
  | 'formulas'
  | 'history'
  | 'usageStats'
  | 'images'
  | 'appearance'
  | 'prompts'
  | 'inputHistory'
  | 'llmConfig'
  | 'webDavConfig'

export interface ResetSectionMeta {
  key: ResetSectionKey
  label: string
  /** 是否属于「配置类」（默认不勾，需要用户自己伸手） */
  configLike: boolean
}

/**
 * 展示顺序 = 执行顺序，与 Rust `ResetSection::ALL_ORDER` 一致。
 *
 * 先把本机数据排前面，配置类（模型、WebDAV）排最后。
 */
export const RESET_SECTIONS: readonly ResetSectionMeta[] = [
  { key: 'formulas', label: '公式与版本', configLike: false },
  { key: 'history', label: '计算历史', configLike: false },
  { key: 'usageStats', label: 'Token 用量记录', configLike: false },
  { key: 'images', label: '图片缓存', configLike: false },
  { key: 'appearance', label: '外观（主题 / 背景 / 文字色）', configLike: false },
  { key: 'prompts', label: '提示词与提醒词', configLike: false },
  { key: 'inputHistory', label: '输入历史与草稿', configLike: false },
  { key: 'llmConfig', label: 'AI 模型配置与密钥', configLike: true },
  { key: 'webDavConfig', label: 'WebDAV 配置', configLike: true },
] as const

// =============================================================================
// 选择
// =============================================================================

/** 与 Rust `ResetSelection` 同形（9 个布尔） */
export interface ResetSelection {
  formulas: boolean
  history: boolean
  usageStats: boolean
  images: boolean
  appearance: boolean
  prompts: boolean
  inputHistory: boolean
  llmConfig: boolean
  webDavConfig: boolean
}

/**
 * 默认勾选：前 7 项数据类全勾，`llmConfig` / `webDavConfig` **不勾**。
 *
 * 源项目注释：「要清得自己伸手」—— 把模型配置与云端账号一起清掉
 * 是很容易后悔的操作。
 */
export function defaultResetSelection(): ResetSelection {
  return {
    formulas: true,
    history: true,
    usageStats: true,
    images: true,
    appearance: true,
    prompts: true,
    inputHistory: true,
    llmConfig: false,
    webDavConfig: false,
  }
}

/** 全不勾 */
export function emptyResetSelection(): ResetSelection {
  return {
    formulas: false,
    history: false,
    usageStats: false,
    images: false,
    appearance: false,
    prompts: false,
    inputHistory: false,
    llmConfig: false,
    webDavConfig: false,
  }
}

/** 已勾选的类别（顺序即执行顺序） */
export function selectedSections(sel: ResetSelection): ResetSectionKey[] {
  return RESET_SECTIONS.filter((s) => sel[s.key]).map((s) => s.key)
}

/** 是否有任何一项被勾选 */
export function hasAnySelected(sel: ResetSelection): boolean {
  return selectedSections(sel).length > 0
}

// =============================================================================
// 存量与结果
// =============================================================================

/** `reset_counts` 的返回：本机现在的存量 */
export interface ResetCounts {
  formulas: number
  favorites: number
  versions: number
  history: number
  usageStats: number
  images: number
  imageBytes: number
  llmProfiles: number
  apiKeys: number
  webDavConfigured: boolean
  hasBackground: boolean
}

/** `reset_execute` 的返回：这次实际清掉的量 */
export interface ResetSummary {
  formulas: number
  favorites: number
  versions: number
  history: number
  usageStats: number
  images: number
  background: boolean
  preferences: number
  llmConfig: boolean
  apiKeys: number
  webDavConfig: boolean
}

// =============================================================================
// 展示
// =============================================================================

/** 字节数 → 人类可读（与 Rust `format_bytes` 同口径：1024 进制） */
export function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GB`
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KB`
  return `${bytes} B`
}

/**
 * 每行右侧的存量说明 —— Rust `ResetCounts::subtitle_of` 的镜像。
 *
 * 数量为 0 时给「暂无…」而不是「0 条」：0 条会让用户以为
 * 「有这一类数据但里面是空的」，而实际是压根没数据。
 */
export function subtitleOf(c: ResetCounts, key: ResetSectionKey): string {
  switch (key) {
    case 'formulas': {
      const parts: string[] = []
      if (c.formulas > 0) parts.push(`${c.formulas} 公式`)
      if (c.favorites > 0) parts.push(`${c.favorites} 收藏`)
      if (c.versions > 0) parts.push(`${c.versions} 版本`)
      return parts.length > 0 ? parts.join(' · ') : '暂无公式数据'
    }
    case 'history':
      return c.history === 0 ? '暂无计算历史' : `${c.history} 记录`
    case 'usageStats':
      return c.usageStats === 0 ? '暂无用量记录' : `${c.usageStats} 记录`
    case 'images':
      return c.images === 0 ? '暂无插图' : `${c.images} 张 · ${formatBytes(c.imageBytes)}`
    case 'appearance':
      return c.hasBackground
        ? '已设背景图（并重置主题 / 文字颜色 / 透明度）'
        : '重置主题 / 文字颜色 / 透明度'
    case 'prompts':
      return '回退到内置提示词与提醒词'
    case 'inputHistory':
      return '搜索 / 精炼 / 模型测试的输入历史与草稿'
    case 'llmConfig':
      if (c.llmProfiles === 0) return '回到内置三家预设'
      return c.apiKeys > 0
        ? `${c.llmProfiles} 个模型 · ${c.apiKeys} 个密钥（回到内置三家预设）`
        : `${c.llmProfiles} 个模型（回到内置三家预设）`
    case 'webDavConfig':
      return c.webDavConfigured ? '已配置（地址 / 账号 / 密码一并清除）' : '当前未配置'
  }
}

/** 一次重置的摘要文案（Rust `ResetSummary::describe` 的镜像） */
export function describeSummary(s: ResetSummary): string {
  const parts: string[] = []
  if (s.formulas > 0) parts.push(`公式 ${s.formulas} 条`)
  if (s.favorites > 0) parts.push(`收藏 ${s.favorites} 条`)
  if (s.versions > 0) parts.push(`版本 ${s.versions} 条`)
  if (s.history > 0) parts.push(`历史 ${s.history} 条`)
  if (s.usageStats > 0) parts.push(`用量记录 ${s.usageStats} 条`)
  if (s.images > 0) parts.push(`插图 ${s.images} 张`)
  if (s.background) parts.push('背景图')
  if (s.preferences > 0) parts.push(`设置 ${s.preferences} 项`)
  if (s.llmConfig) parts.push('AI 模型配置')
  if (s.apiKeys > 0) parts.push(`密钥 ${s.apiKeys} 个`)
  if (s.webDavConfig) parts.push('WebDAV 配置')
  return parts.length > 0 ? `已重置：${parts.join(' · ')}` : '没有可重置的内容'
}
