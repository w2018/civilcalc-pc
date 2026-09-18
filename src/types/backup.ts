/**
 * 组 14：备份与 WebDAV（本地备份 3 命令 + 云端 11 命令）。
 *
 * 见 docs/08-IPC契约.md §3 组 14、docs/04-数据契约.md §5。
 *
 * ## 🔴 两个 `password` 参数含义**不同**
 *
 * | 出现在 | 含义 |
 * |---|---|
 * | `webdavConfigSave(config, password?)` | **WebDAV 账号密码**（存 keyring） |
 * | `webdavUpload` / `webdavInspect` / `webdavDownloadImport` 的 `password?` | **备份包加密密码**（不存） |
 *
 * 前者是「连得上服务器」，后者是「打得开包」。同名不同物 ——
 * 传错不会报错，只会「连不上」或「打不开」，很难查。
 *
 * ## 备份包是**跨端契约**
 *
 * Android 版也能读这些包，所以：
 * - `BackupSection` 是 `SCREAMING_SNAKE_CASE`（枚举名进包，不能改）
 * - `verified` / `favorite` 在包里是 `0/1` 而不是 `boolean`
 */

// =============================================================================
// 备份范围与类别
// =============================================================================

/**
 * 上传/导出时勾选的备份范围。
 *
 * 图片三档的默认值：**被公式引用的默认勾**（不勾则公式里的内联图恢复后是
 * 「已删除」占位），被对话/历史引用的与未被引用的**默认不勾** —— 按需选择，包体更小。
 */
export interface BackupSelection {
  /** 公式 + 收藏 + 版本链 */
  formulas: boolean
  history: boolean
  /** 偏好（主题 / 提示词 / 提醒词 / 导出选项 / 输入历史 / 模型测试对话等） */
  preferences: boolean
  usageStats: boolean
  /** 真实生效的 AI 模型配置（**不含密钥**） */
  llmConfig: boolean
  /** 模型密钥：默认不勾，勾选后**明文**写入备份包 */
  includeApiKeys: boolean
  includeFormulaImages: boolean
  includeHistoryImages: boolean
  includeUnreferencedImages: boolean
}

/** 默认勾选（对齐源项目的初始状态） */
export function defaultBackupSelection(): BackupSelection {
  return {
    formulas: true,
    history: true,
    preferences: true,
    usageStats: true,
    llmConfig: true,
    includeApiKeys: false,
    includeFormulaImages: true,
    includeHistoryImages: false,
    includeUnreferencedImages: false,
  }
}

/**
 * 备份包里的一个数据类别。
 *
 * ⚠️ **枚举名进备份包**（`SCREAMING_SNAKE_CASE`），与 Android 端逐字一致，不能改。
 */
export type BackupSection =
  | 'FORMULAS'
  | 'HISTORY'
  | 'PREFERENCES'
  | 'USAGE_STATS'
  | 'LLM_CONFIG'
  | 'API_KEYS'
  | 'IMAGES'
  | 'BACKGROUND'

/** 类别 → 中文名（确认弹窗里列「将清空哪些」用） */
export function sectionLabel(s: BackupSection): string {
  switch (s) {
    case 'FORMULAS':
      return '公式与收藏'
    case 'HISTORY':
      return '计算历史'
    case 'PREFERENCES':
      return '偏好设置'
    case 'USAGE_STATS':
      return '用量统计'
    case 'LLM_CONFIG':
      return 'AI 模型配置'
    case 'API_KEYS':
      return '模型密钥'
    case 'IMAGES':
      return '图片'
    case 'BACKGROUND':
      return '背景图'
    default:
      return s
  }
}

// =============================================================================
// 摘要与报告
// =============================================================================

/**
 * 备份内容摘要。
 *
 * **既是导出时的「我放了什么」，也是导入后的「我写了什么」** —— 同一个类型两处用。
 */
export interface BackupSummary {
  formulas: number
  history: number
  favorites: number
  versions: number
  usageStats: number
  /** 偏好项数（strings + ints） */
  preferences: number
  llmConfig: boolean
  apiKeys: number
  images: number
  background: boolean
}

/** 空摘要（未解码的加密包会返回它） */
export function emptyBackupSummary(): BackupSummary {
  return {
    formulas: 0,
    history: 0,
    favorites: 0,
    versions: 0,
    usageStats: 0,
    preferences: 0,
    llmConfig: false,
    apiKeys: 0,
    images: 0,
    background: false,
  }
}

/** 摘要是否全空（用于「这个包什么都没带」的提示） */
export function isSummaryEmpty(s: BackupSummary): boolean {
  return (
    s.formulas === 0 &&
    s.history === 0 &&
    s.favorites === 0 &&
    s.versions === 0 &&
    s.usageStats === 0 &&
    s.preferences === 0 &&
    !s.llmConfig &&
    s.apiKeys === 0 &&
    s.images === 0 &&
    !s.background
  )
}

/**
 * 摘要 → 一行中文描述（完成提示用）。
 *
 * 只列**非零项**；全空时返回「空备份」。
 */
export function describeSummary(s: BackupSummary): string {
  const parts: string[] = []
  if (s.formulas > 0) parts.push(`公式 ${s.formulas}`)
  if (s.favorites > 0) parts.push(`收藏 ${s.favorites}`)
  if (s.versions > 0) parts.push(`版本 ${s.versions}`)
  if (s.history > 0) parts.push(`历史 ${s.history}`)
  if (s.usageStats > 0) parts.push(`用量 ${s.usageStats}`)
  if (s.preferences > 0) parts.push(`偏好 ${s.preferences}`)
  if (s.llmConfig) parts.push('模型配置')
  if (s.apiKeys > 0) parts.push(`密钥 ${s.apiKeys}`)
  if (s.images > 0) parts.push(`图片 ${s.images}`)
  if (s.background) parts.push('背景图')
  return parts.length > 0 ? parts.join(' · ') : '空备份'
}

/** 本地导出结果 */
export interface LocalExportResult {
  /** **最终实际路径**（同名冲突时带 `(2)` 这类序号）—— 打开文件夹要用它 */
  path: string
  displayName: string
  sizeBytes: number
  summary: BackupSummary
}

/**
 * 只解析清单不导入的结果（二次确认弹窗用）。
 *
 * ## 🔴 `decoded === false` 是**正常返回值**，不是错误
 *
 * 加密包在**没给密码 / 密码不对**时仍返回成功，只是 `decoded` 为 `false`
 * 且 `summary` / `sections` 为空。前端据此弹「请输入密码」并**带上密码重调**。
 *
 * 其它失败（坏包、截断）才会抛错。
 */
export interface BackupInspectResult {
  fileName: string
  sizeBytes: number
  /** 加密包（读文件头魔数判定，**不需要密码**） */
  encrypted: boolean
  /** 清单是否已解出（见上方说明） */
  decoded: boolean
  /** 包的创建时间；未解码时为 0 */
  createdAt: number
  /** 打包时的应用版本名；未解码时为空串 */
  appVersionName: string
  summary: BackupSummary
  /** 包内实际包含的类别；未解码时为空 */
  sections: BackupSection[]
}

/** 导入口径 */
export type ImportMode = 'merge' | 'replace'

/** 导入结果 */
export interface ImportReport {
  /** 实际使用的口径 */
  mode: ImportMode
  /** **实际写入的**条数（图片是真实恢复数，不是包里声明的数） */
  summary: BackupSummary
}

// =============================================================================
// WebDAV
// =============================================================================

export type WebDavPreset = 'NUTSTORE' | 'CUSTOM'

export const DEFAULT_WEBDAV_BASE_URL = 'https://dav.jianguoyun.com/dav'
export const DEFAULT_WEBDAV_REMOTE_DIR = 'civilcalc'
/** 远端备份列表最多展示条数（与后端 `LIST_LIMIT` 一致） */
export const WEBDAV_LIST_LIMIT = 20

/** ⚠️ **不含密码** —— 密码单独存 keyring */
export interface WebDavConfig {
  baseUrl: string
  username: string
  remoteDir: string
  preset: WebDavPreset
}

export function defaultWebDavConfig(): WebDavConfig {
  return {
    baseUrl: DEFAULT_WEBDAV_BASE_URL,
    username: '',
    remoteDir: DEFAULT_WEBDAV_REMOTE_DIR,
    preset: 'NUTSTORE',
  }
}

/** 预设 → 默认地址（自定义留空由用户填） */
export function defaultBaseUrlOf(preset: WebDavPreset): string {
  return preset === 'NUTSTORE' ? DEFAULT_WEBDAV_BASE_URL : ''
}

/**
 * 连通性测试结果。
 *
 * 🔴 **本地配置问题抛错，连不上返回 `ok: false`** —— 两者必须区分：
 * 前者要用户去改配置，后者要用户查网络。都做成错误的话前端只能显示一句红字。
 */
export interface WebDavTestResult {
  ok: boolean
  /** 归一化后的远端目录 URL（**成败都给**，便于核对连的是哪个地址） */
  dirUrl: string
  message: string
}

/**
 * 远端一条备份记录。
 *
 * ⚠️ `timestampMs` 与 `lastModifiedMs` **不同**：
 * 前者优先取**文件名里的时间戳**（本机生成时写进去的，更可信），
 * 后者是服务端给的（有的网盘不返回，或返回 UTC 却标成本地）。
 * 展示与排序用 `timestampMs`。
 */
export interface RemoteBackup {
  name: string
  sizeBytes: number
  lastModifiedMs: number
  timestampMs: number
}

/**
 * 远端列表结果。
 *
 * ⚠️ 契约（`docs/05`）写的是 `RemoteEntry[]`，但那样**丢了 `total`** ——
 * 而界面要提示「还有 N 条更早的」。`backups` 已按时间倒序并截断到
 * {@link WEBDAV_LIST_LIMIT}，`total` 是**服务器上的总数**。
 */
export interface RemoteListResult {
  backups: RemoteBackup[]
  total: number
}

/** 上传成功的结果 */
export interface UploadResult {
  name: string
  sizeBytes: number
  /** 最终上传到的 URL（展示与排错用） */
  remoteUrl: string
}

// =============================================================================
// 事件
// =============================================================================

/** 上传/下载进度 */
export const BACKUP_PROGRESS_EVENT = 'backup://progress'
/** 用户取消（配合 `webdavCancel`） */
export const BACKUP_CANCELLED_EVENT = 'backup://cancelled'

/** `backup://progress` 的载荷 */
export interface BackupProgress {
  /** `pack` / `upload` / `download` / `import` */
  stage: string
  current: number
  /**
   * 总字节数；**服务端不给 `Content-Length` 时为 0**（进度未知）。
   *
   * ⚠️ 后端刻意不用 `-1`：`current/total` 都是无符号数，负数要额外约定。
   */
  total: number
}

/**
 * 阶段的中文名（进度提示用）。
 *
 * 未知阶段原样返回 —— 后端加阶段时前端不会显示空白。
 */
export function backupStageLabel(stage: string): string {
  switch (stage) {
    case 'pack':
      return '打包'
    case 'upload':
      return '上传'
    case 'download':
      return '下载'
    case 'import':
      return '导入'
    default:
      return stage
  }
}

/**
 * 进度百分比（0~100 整数）。
 *
 * `total <= 0` 时返回 **0** —— 调用方据此显示「已传输 x MB」而不是百分比
 * （网盘不给长度时百分比没有意义）。
 */
export function backupPercent(p: BackupProgress): number {
  if (p.total <= 0) return 0
  return Math.min(100, Math.round((p.current / p.total) * 100))
}

/** 字节数 → 人类可读（与后端同一口径：1024 进制） */
export function humanSize(bytes: number): string {
  const KB = 1024
  const MB = KB * 1024
  if (bytes < KB) return `${Math.max(0, Math.round(bytes))} B`
  if (bytes < MB) return `${(bytes / KB).toFixed(1)} KB`
  return `${(bytes / MB).toFixed(1)} MB`
}
