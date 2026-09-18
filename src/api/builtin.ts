/**
 * 组 2：内置公式库。
 *
 * 内置库是**编译进二进制**的只读资产（55 条，ADR-020），
 * 不从网络或磁盘加载。
 *
 * 见 docs/08-IPC契约.md §3 组 2。
 */

import { invoke } from './invoke'
import type { FormulaSchema } from '@/types/domain'
import type { BuiltinStatus } from '@/types/system'

/**
 * 内置公式原始 JSON（55 条）。
 *
 * 首屏渲染**优先用这个** —— 省一次「命令 → 结构体 → 序列化」的往返。
 */
export function builtinFormulasJson(): Promise<string> {
  return invoke<string>('builtin_formulas_json')
}

/** 内置公式（已解析 + 已过来源校验） */
export function builtinFormulas(): Promise<FormulaSchema[]> {
  return invoke<FormulaSchema[]>('builtin_formulas')
}

/** 内置库状态：解析条数 + 跳过清单 + 数据库实际条数 */
export function builtinStatus(): Promise<BuiltinStatus> {
  return invoke<BuiltinStatus>('builtin_status')
}

/**
 * 重新解析并校验内置库（排障用）。
 *
 * ⚠️ **不写数据库** —— 播种只在启动时做一次。
 * 用户删掉的某条内置公式不会被这里加回来。
 */
export function builtinReload(): Promise<BuiltinStatus> {
  return invoke<BuiltinStatus>('builtin_reload')
}
