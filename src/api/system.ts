/**
 * 组 1：系统与配置。
 *
 * 见 docs/08-IPC契约.md §3 组 1。
 */

import { invoke } from './invoke'
import type { AppInfo, ConfigSection, ConfigSnapshot } from '@/types/system'

/** 应用信息（"关于"页 + 排障）。**不会失败** —— 库读不到时字段降级 */
export function appInfo(): Promise<AppInfo> {
  return invoke<AppInfo>('app_info')
}

/**
 * 显式等待数据库就绪。
 *
 * 应用显示窗口即已初始化完成；此命令供前端有一个明确的同步点。
 * 返回 `user_version`（当前为 1）。
 */
export function initDb(): Promise<number> {
  return invoke<number>('init_db')
}

/** 读全部偏好 */
export function configGet(): Promise<ConfigSnapshot> {
  return invoke<ConfigSnapshot>('config_get')
}

/**
 * 保存全部偏好。
 *
 * ⚠️ **整体替换**（不是合并）—— 必须先 `configGet` 再改再存，
 * 只传要改的键会把其他键清掉。
 */
export function configSave(snapshot: ConfigSnapshot): Promise<void> {
  return invoke<void>('config_save', { snapshot })
}

/**
 * 按区块重置偏好（删键 → 回落默认值）。
 *
 * @returns 删除的键数
 */
export function configResetSection(section: ConfigSection): Promise<number> {
  return invoke<number>('config_reset_section', { section })
}
