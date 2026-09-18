/**
 * 组 7a：收藏。
 *
 * 见 docs/08-IPC契约.md §3 组 7。
 */

import { invoke } from './invoke'
import type { FormulaSchema } from '@/types/domain'

export function isFavorite(id: string): Promise<boolean> {
  return invoke<boolean>('is_favorite', { id })
}

/**
 * 切换收藏。
 *
 * @returns **切换后**的状态（不要用 `!之前的值` 猜）
 */
export function toggleFavorite(id: string): Promise<boolean> {
  return invoke<boolean>('toggle_favorite', { id })
}

/** 收藏列表（按**收藏时间倒序**） */
export function favoritesList(): Promise<FormulaSchema[]> {
  return invoke<FormulaSchema[]>('favorites_list')
}
