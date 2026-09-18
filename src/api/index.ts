/**
 * IPC 出口统一索引。
 *
 * 视图与组件**只从这里 import**（或从各自的域模块），
 * 不直接使用 `@tauri-apps/api`（见 docs/05 §3.1.2）。
 *
 * 用法：
 * ```ts
 * import { formulaApi, systemApi } from '@/api'
 * const info = await systemApi.appInfo()
 * const f = await formulaApi.formulaGet('usr:1')
 * ```
 *
 * 或按需具名导入：
 * ```ts
 * import { formulaGet, formulaSave } from '@/api/formula'
 * ```
 */

export * as aiApi from './ai'
export * as appearanceApi from './appearance'
export * as backupApi from './backup'
export * as builtinApi from './builtin'
export * as displayApi from './display'
export * as evalApi from './eval'
export * as excelApi from './excel'
export * as favoriteApi from './favorite'
export * as formulaApi from './formula'
export * as historyApi from './history'
export * as imageApi from './image'
export * as llmApi from './llm'
export * as modelTestApi from './modelTest'
export * as reportApi from './report'
export * as resetApi from './reset'
export * as searchApi from './search'
export * as systemApi from './system'
export * as updateApi from './update'
export * as verifyApi from './verify'
export * as usageApi from './usage'
export * as versionApi from './version'
export * as webdavApi from './webdav'

export { invoke, normalizeError } from './invoke'
