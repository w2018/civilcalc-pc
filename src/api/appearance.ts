/**
 * 组 14a：外观与背景图。
 *
 * 见 docs/08-IPC契约.md §3 组 14。
 */

import { invoke } from './invoke'
import type { Appearance, BackgroundInfo } from '@/types/system'

/** 读外观聚合视图（主题 + 背景 + 文字色） */
export function appearanceGet(): Promise<Appearance> {
  return invoke<Appearance>('appearance_get')
}

/** 保存外观。⚠️ 是**整体覆盖**，请先 `appearanceGet` 再改 */
export function appearanceSave(appearance: Appearance): Promise<void> {
  return invoke<void>('appearance_save', { appearance })
}

/** 读背景信息 */
export function backgroundGet(): Promise<BackgroundInfo> {
  return invoke<BackgroundInfo>('background_get')
}

/**
 * 设置背景图（把 `path` 指向的图片**复制**进应用数据目录）。
 *
 * `transparency` 会被夹到 `0~100`。
 */
export function backgroundSave(path: string, transparency: number): Promise<void> {
  return invoke<void>('background_save', { path, transparency })
}

/**
 * 清除背景图，并**一并重置文字色与透明度**（源项目行为）。
 *
 * 只删背景图文件本身，不动其他数据。
 */
export function backgroundClear(): Promise<void> {
  return invoke<void>('background_clear')
}

/** 默认背景透明度（22） */
export function backgroundDefaultTransparency(): Promise<number> {
  return invoke<number>('background_default_transparency')
}
