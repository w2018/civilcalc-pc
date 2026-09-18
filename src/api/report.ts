/**
 * 组 13：报告与导出。
 *
 * 见 docs/08-IPC契约.md §3 组 13。
 *
 * ## ⚠️ 预览与导出都**不写计算历史**
 *
 * 它们不是「确认计算」。用户点 5 次预览不该灌 5 条历史。
 *
 * ## ⚠️ 用返回的 `path`，不要用你传的路径
 *
 * 同名冲突时后端会加序号（**从 `(2)` 起**）——
 * 必须用返回值里的 `path` 去「打开文件」。
 */

import { invoke } from './invoke'
import type { FormulaSchema } from '@/types/domain'
import type {
  DocumentExportResult,
  DocumentOutputTarget,
  ExportOptions,
} from '@/types/report'

/** 读持久化的导出选项（首次使用返回默认值：全开） */
export function exportOptionsGet(): Promise<ExportOptions> {
  return invoke<ExportOptions>('export_options_get')
}

/** 保存导出选项（**整体覆盖**，请先 `exportOptionsGet` 再改） */
export function exportOptionsSave(options: ExportOptions): Promise<void> {
  return invoke<void>('export_options_save', { options })
}

/**
 * 渲染 HTML 预览（ADR-018）。
 *
 * 返回**完整 HTML 文档**，塞进 `iframe.srcdoc` 即可。
 *
 * @param options 不传则用持久化的选项
 */
export function reportPreview(
  formulaId: string,
  inputs: Record<string, number>,
  options?: ExportOptions,
): Promise<string> {
  return invoke<string>('report_preview', { formulaId, inputs, options })
}

/**
 * 导出 `.docx` 计算书。
 *
 * 过程中会发 `export://progress` 事件（见 `types/report.ts` 的
 * `EXPORT_PROGRESS_EVENT` / `progressPercent`）。
 *
 * @param options 不传则用持久化的选项
 * @returns `path` 是**最终实际路径**（可能带序号），用它打开文件
 */
export function reportExportDocx(
  formulaId: string,
  inputs: Record<string, number>,
  target: DocumentOutputTarget,
  options?: ExportOptions,
): Promise<DocumentExportResult> {
  return invoke<DocumentExportResult>('report_export_docx', {
    formulaId,
    inputs,
    options,
    target,
  })
}

/**
 * 在系统文件管理器里定位文件。
 *
 * 文件不存在时抛 `notFound` —— 用户可能刚删了它，
 * 静默什么都不做会让人以为按钮坏了。
 */
export function reportReveal(path: string): Promise<void> {
  return invoke<void>('report_reveal', { path })
}

/**
 * 便捷：导出到应用缓存并返回结果（用于「快速预览文件」）。
 *
 * ⚠️ 缓存目录**会被系统清理**，不适合放用户要留存的成果。
 */
export function reportExportToCache(
  formulaId: string,
  inputs: Record<string, number>,
  options?: ExportOptions,
): Promise<DocumentExportResult> {
  return reportExportDocx(formulaId, inputs, { kind: 'privateCache' }, options)
}

/** 便捷：导出到系统「下载」 */
export function reportExportToDownloads(
  formulaId: string,
  inputs: Record<string, number>,
  options?: ExportOptions,
): Promise<DocumentExportResult> {
  return reportExportDocx(formulaId, inputs, { kind: 'scopedDownloads' }, options)
}

/** 未使用的类型引用占位（避免 lint 误删 `FormulaSchema` 导入） */
export type ReportSchema = FormulaSchema
