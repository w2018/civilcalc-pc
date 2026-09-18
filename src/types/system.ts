/**
 * 系统与配置类型 —— 与 Rust `src-tauri/src/{lib,config,commands/*}.rs` 对应。
 *
 * 见 docs/08-IPC契约.md §2.4。
 */

import type { SourceKind } from './domain'

// =============================================================================
// 应用信息
// =============================================================================

export interface AppInfo {
  version: string
  productName: string
  identifier: string
  dbPath: string
  appDataDir: string
  appCacheDir: string
  /** 日志目录（由 tauri-plugin-log 创建） */
  logsDir: string
  /** 编译进二进制的内置公式条数（固定 55） */
  builtinFormulaCount: number
  /** 库中公式总行数（内置播种 + 用户自建） */
  formulaCount: number
  /** 数据库 `user_version`；**-1 表示读取失败** */
  dbSchemaVersion: number
}

// =============================================================================
// 偏好
// =============================================================================

/**
 * 偏好快照 —— 与备份包 `BackupPrefs` 同形。
 *
 * ⚠️ `config_save` 是**整体替换**（不是合并）：正确用法是
 * `config_get` → 改 → `config_save`。
 */
export interface ConfigSnapshot {
  strings: Record<string, string>
  ints: Record<string, number>
}

export type ThemeMode = 'SYSTEM' | 'LIGHT' | 'DARK'

/** `config_reset_section` 的合法取值 */
export type ConfigSection =
  | 'APPEARANCE'
  | 'PROMPTS'
  | 'EXPORT'
  | 'SEARCH'
  | 'DRAFTS'
  | 'MODEL_TEST'
  | 'REFINE'
  | 'WEBDAV_STATS'

/** 区块 → 中文名（设置页「重置」用） */
export const CONFIG_SECTION_LABELS: Record<ConfigSection, string> = {
  APPEARANCE: '外观（主题 / 背景 / 文字色）',
  PROMPTS: '提示词与提醒词',
  EXPORT: '导出选项',
  SEARCH: '搜索历史',
  DRAFTS: '公式参数草稿',
  MODEL_TEST: '模型测试',
  REFINE: '微调输入历史',
  WEBDAV_STATS: '云端备份统计',
}

// ---- 偏好键名（与 Rust config.rs 的常量逐字一致） ----

export const PREF_KEYS = {
  theme: 'theme_mode',
  promptA: 'prompt_a',
  defaultReminder: 'default_reminder',
  searchQuery: 'search_query',
  modelTestInput: 'model_test_input',
  drafts: 'formula_drafts',
  background: 'background_image',
  backgroundTransparency: 'background_transparency',
  textColor: 'text_color',
  backgroundColor: 'background_color',
  showPrompts: 'show_prompts_section',
  searchHistory: 'search_input_history',
  refineHistory: 'refine_input_history',
  modelTestHistory: 'model_test_input_history',
  modelTestConversation: 'model_test_conversation',
  testContextSize: 'model_test_context_size',
  testCompressThreshold: 'model_test_compress_threshold',
  testCompressNotice: 'model_test_compress_notice',
  testSystemPrompt: 'model_test_system_prompt',
  generateExplanation: 'generate_explanation',
  exportOptions: 'export_options',
  webdavLastBackup: 'webdav_last_backup_at',
  webdavBackupCount: 'webdav_backup_count',
} as const

// ---- 默认值（与 Rust 常量逐字一致） ----

export const DEFAULT_BG_TRANSPARENCY = 22
export const DEFAULT_TEST_CONTEXT_SIZE = 200_000
export const DEFAULT_TEST_COMPRESS_THRESHOLD = 80
export const TEXT_COLOR_AUTO = 'AUTO'

/** 从快照读主题（缺省 SYSTEM） */
export function readTheme(snap: ConfigSnapshot): ThemeMode {
  const v = snap.strings[PREF_KEYS.theme]
  return v === 'LIGHT' || v === 'DARK' ? v : 'SYSTEM'
}

/** 从快照读背景透明度（夹紧 0~100，缺省 22） */
export function readBgTransparency(snap: ConfigSnapshot): number {
  const v = snap.ints[PREF_KEYS.backgroundTransparency]
  if (v === undefined) return DEFAULT_BG_TRANSPARENCY
  return Math.min(100, Math.max(0, v))
}

/** 从快照读是否设置了背景图 */
export function readHasBackground(snap: ConfigSnapshot): boolean {
  return snap.strings[PREF_KEYS.background] === '1'
}

// =============================================================================
// 内置公式库
// =============================================================================

export interface BuiltinStatus {
  total: number
  valid: number
  /** 校验被跳过的条目，格式 `"id: 原因"` */
  skipped: string[]
  dbFormulaCount: number
}

// =============================================================================
// 公式筛选
// =============================================================================

export interface FormulaFilter {
  /** 领域精确匹配 */
  domain?: string | null
  sourceKind?: SourceKind | null
  favoriteOnly: boolean
}

export function emptyFilter(): FormulaFilter {
  return { domain: null, sourceKind: null, favoriteOnly: false }
}

// =============================================================================
// 外观
// =============================================================================

/** `appearance_get` 的返回（主题 + 背景 + 文字色的**聚合视图**） */
export interface Appearance {
  themeMode: ThemeMode
  hasBackground: boolean
  /** 0~100 */
  backgroundTransparency: number
  /** `AUTO` 或具体色值 */
  textColor: string
  /**
   * 自定义页面底色（`#RRGGBB`）；`null` = 用主题默认底色。
   *
   * ⚠️ 这个字段是后补的：此前「页面底色」只改内存与 DOM、从不落盘，
   * 重启就丢、「重置软件」也回不到默认。
   */
  backgroundColor: string | null
}

/**
 * `background_get` 的返回。
 *
 * ⚠️ 与 `Appearance` 的差别：这里给的是**背景图文件是否真的存在**。
 * 偏好里标记为「有背景」但文件被用户删掉时，`hasBackground` 会返回 `false`
 * （后端已做兜底，前端不要再自己 `exists` 检查）。
 */
export interface BackgroundInfo {
  hasBackground: boolean
  /** 0~100 */
  backgroundTransparency: number
  /** **存储**的文字色，可能是 `AUTO`。实际生效色由主题/背景亮度决定 */
  textColor: string
  /** 背景图绝对路径；未设置或文件丢失时为 `null` */
  imagePath: string | null
}

/** 由快照拼出外观视图（前端本地用，避免多一次 IPC） */
export function appearanceOf(snap: ConfigSnapshot): Appearance {
  return {
    themeMode: readTheme(snap),
    hasBackground: readHasBackground(snap),
    backgroundTransparency: readBgTransparency(snap),
    textColor: snap.strings[PREF_KEYS.textColor] ?? TEXT_COLOR_AUTO,
    backgroundColor: snap.strings[PREF_KEYS.backgroundColor]?.trim() || null,
  }
}

// =============================================================================
// Token 用量统计
// =============================================================================

export type UsageGroupBy = 'model' | 'day'

export interface UsageSummary {
  modelLabel: string
  totalPrompt: number
  totalCompletion: number
  totalAll: number
  totalCached: number
  totalReasoning: number
  callCount: number
}

export interface UsageDaySummary {
  /** 本地日期 `YYYY-MM-DD` */
  day: string
  totalPrompt: number
  totalCompletion: number
  totalAll: number
  totalCached: number
  totalReasoning: number
  callCount: number
}

export interface UsageTotals {
  totalPrompt: number
  totalCompletion: number
  totalAll: number
  totalCached: number
  totalReasoning: number
  callCount: number
}

/** `usage_stats` 的返回：三种视图一次给全 */
export interface UsageStats {
  /** 请求的分组方式（回显） */
  groupBy: UsageGroupBy
  byModel: UsageSummary[]
  byDay: UsageDaySummary[]
  total: UsageTotals
}
