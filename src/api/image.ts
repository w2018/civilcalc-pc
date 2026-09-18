/**
 * 组 14：AI 输入图片缓存（6 个命令）。
 *
 * 见 docs/08-IPC契约.md §3 组 14。
 *
 * ## 🔴 两个保存入口走**同一套**处理管线
 *
 * 读外部图片 → 处理 → 落盘：EXIF 方向修正、长边缩放到 1024、统一转 JPEG q80。
 * 处理完按**内容 SHA-1 去重**写入 —— 同一张图即使从不同入口导入也只存一份。
 *
 * ## ref_key 从哪来
 *
 * `image_save_from_path` 没有对应的 `addRef` / `releaseRef` 命令（契约未列），
 * 所以每导入一次就用**规范化后的源路径**作 ref_key。同一文件重复导入只计 1 次引用。
 *
 * 剪贴板图片**没有源路径**（`imageSaveFromBytes`），后端默认用内容哈希
 * （`clip:<sha1>`）作引用键 —— 同一张截图粘两次也只计 1 次引用。
 */

import { invoke } from './invoke'
import type { ImageCacheItem } from '@/types/image'

/** 图片缓存列表（含 `refCount`）。 */
export function imageList(): Promise<ImageCacheItem[]> {
  return invoke<ImageCacheItem[]>('image_list')
}

/** 总占用字节。 */
export function imageTotalSize(): Promise<number> {
  return invoke<number>('image_total_size')
}

/**
 * 强制删除指定图片（文件 + 记录），返回**实际删除数量**。
 *
 * ⚠️ 被引用的图也照删 —— 调用方负责在 UI 上提示影响。
 */
export function imageDelete(ids: string[]): Promise<number> {
  return invoke<number>('image_delete', { ids })
}

/**
 * 按 ID 取回 data URL（全屏预览 / 九宫格缩略图）。
 * 记录或文件缺失（已被图片管理删除等）返回 `null`。
 */
export function imageLoadDataUrl(id: string): Promise<string | null> {
  return invoke<string | null>('image_load_data_url', { id })
}

/**
 * 保存外部图片（长边 1024 / JPEG q80 / EXIF 方向），返回图片 id。
 *
 * @param path 系统绝对路径（来自文件选择器）
 */
export function imageSaveFromPath(path: string): Promise<string> {
  return invoke<string>('image_save_from_path', { path })
}

/**
 * 保存**字节**图片（剪贴板粘贴），返回图片 id。
 *
 * 与 {@link imageSaveFromPath} 走完全相同的处理管线，
 * 只是入口不同 —— 剪贴板里的图没有文件路径可用。
 *
 * @param data data URL（`data:image/png;base64,...`）或裸 base64
 * @param refKey 自定义引用键；不传则由后端按内容哈希生成（`clip:<sha1>`）
 */
export function imageSaveFromBytes(data: string, refKey?: string): Promise<string> {
  return invoke<string>('image_save_from_bytes', { data, refKey })
}
