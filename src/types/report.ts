/**
 * 组 13：报告与导出（计算书）。
 *
 * 见 docs/08-IPC契约.md §3 组 13 与 docs/04-数据契约.md §5。
 *
 * ## 「导出选项」取代「模板」（ADR-023）
 *
 * 源项目原本有模板 CRUD，但实际用下来**建模板＝建文件夹** ——
 * 只是给一组章节起个名字，没有额外能力，用户判定无存在价值。
 * 现在改成「导出时直接勾选 + 结果持久化」。
 * 因此**没有 `template_*` 命令**。
 *
 * ## 预览与导出是两套渲染
 *
 * `report_preview` 返回 HTML，`report_export_docx` 生成 Word。
 * 两者**共享章节顺序与标题常量**，但代码不同，存在视觉不一致风险 ——
 * 预览页已内建标注「预览仅供参考，实际以导出的 Word 文件为准」（ADR-018）。
 */

/**
 * 计算书导出选项。7 个勾选 + 免责声明，**持久化到 `AppConfig`**（不是独立表）。
 *
 * 默认**全开** —— 用户不选也能导出一份完整计算书。
 */
export interface ExportOptions {
  /** 封面（主标题 + 计算器名） */
  showCover: boolean
  showFormulaInfo: boolean
  showParamsTable: boolean
  showResultBlock: boolean
  /** 分步计算 */
  showCalculationSteps: boolean
  showExplanation: boolean
  showNotes: boolean
  /** 免责声明；**空串 = 用内置默认**（`本计算书由AI全能计算器自动生成，结果需经注册工程师复核。`） */
  disclaimer: string
}

/** 导出选项全开的默认值（与后端 `ExportOptions::default()` 一致） */
export const DEFAULT_EXPORT_OPTIONS: ExportOptions = {
  showCover: true,
  showFormulaInfo: true,
  showParamsTable: true,
  showResultBlock: true,
  showCalculationSteps: true,
  showExplanation: true,
  showNotes: true,
  disclaimer: '',
}

/**
 * 落盘目标（4 档）。
 *
 * | 档位 | 位置 | 是否自动加序号 |
 * |---|---|---|
 * | `privateCache` | 应用缓存 `reports/`（**会被清理**） | ✅ |
 * | `scopedDownloads` | 系统「下载」 | ✅ |
 * | `chosenDirectory` | 用户选的目录 | ✅ |
 * | `chosenFile` | 用户选的完整路径 | ❌ 用户已在保存对话框确认覆盖 |
 *
 * ⚠️ **序号从 `(2)` 起**（`报告(2).docx`），不是 `(1)`。
 */
export type DocumentOutputTarget =
  | { kind: 'privateCache' }
  | { kind: 'scopedDownloads' }
  | { kind: 'chosenDirectory'; dir: string }
  | { kind: 'chosenFile'; path: string }

/** 一次成功导出的结果 */
export interface DocumentExportResult {
  /**
   * **最终实际写入的路径**。
   *
   * ⚠️ 发生同名冲突时它会带序号 —— 「打开文件 / 打开所在文件夹」
   * **必须用它**，不要用用户原本选的路径（那可能不存在）。
   */
  path: string
  /** 展示用文件名（含扩展名） */
  displayName: string
  sizeBytes: number
}

/** `export://progress` 的载荷 */
export interface ExportProgress {
  /** 阶段：`prepare` / `render` / `write` / `done` */
  stage: string
  current: number
  total: number
}

/** 导出进度事件名 */
export const EXPORT_PROGRESS_EVENT = 'export://progress'

/**
 * 进度百分比（0~100，整数）。
 *
 * `total <= 0` 时返回 0（防御除零）。
 */
export function progressPercent(p: ExportProgress): number {
  if (p.total <= 0) return 0
  return Math.min(100, Math.round((p.current / p.total) * 100))
}

/**
 * 阶段的中文名（进度提示用）。
 *
 * 未知阶段原样返回 —— 后端加阶段时前端不会显示空白。
 */
export function progressStageLabel(stage: string): string {
  switch (stage) {
    case 'prepare':
      return '准备数据'
    case 'render':
      return '生成文档'
    case 'write':
      return '写入文件'
    case 'done':
      return '完成'
    default:
      return stage
  }
}

/** 字节数 → 人类可读（与后端 `human_size` 同一口径：1024 进制） */
export function humanSize(bytes: number): string {
  const KB = 1024
  const MB = KB * 1024
  if (bytes < KB) return `${Math.max(0, Math.round(bytes))} B`
  if (bytes < MB) return `${(bytes / KB).toFixed(1)} KB`
  return `${(bytes / MB).toFixed(1)} MB`
}

/** 是否一个章节都没勾（导出会得到只有封面的空文档） */
export function isEmptySelection(o: ExportOptions): boolean {
  return !(
    o.showFormulaInfo ||
    o.showParamsTable ||
    o.showResultBlock ||
    o.showCalculationSteps ||
    o.showExplanation ||
    o.showNotes
  )
}
