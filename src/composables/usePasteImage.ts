/**
 * 剪贴板图片粘贴导入 —— 需求：「所有的附件添加图片，增加支持复制图片粘贴导入」。
 *
 * ## 为什么剪贴板图片要单独一条链路
 *
 * 「选文件」拿到的是**路径**（`image_save_from_path`），
 * 而剪贴板里的截图 / 复制的图**没有路径** ——
 * 只能把字节读成 data URL，走 `image_save_from_bytes`。
 * 后端两条链路共用同一套处理管线（EXIF 方向 / 长边 1024 / JPEG q80），
 * 所以粘贴进来的图和选进来的图在库里没有区别。
 *
 * ## 只拦「图片」，不拦文字
 *
 * 粘贴事件里可能带的是文本（用户想粘一段需求描述）。这时**必须放行**，
 * 让输入框照常收到文字 —— 一旦无条件 `preventDefault()`，
 * 用户就再也无法往输入框里粘贴文本了。
 *
 * 判据是 `clipboardData.items` 里有没有 `type` 以 `image/` 开头的项。
 *
 * ## 支持一次粘贴多张
 *
 * 有的截图工具会把多张图放进剪贴板。逐个处理，全部完成后一起回调，
 * 这样调用方只需处理一次「新增了 N 张」。
 *
 * ## 挂在 `window` 上而不是元素上
 *
 * 用户可能刚点完按钮、焦点不在输入框里就按 Ctrl+V。
 * 挂在 `window` 上能让整页都能接住粘贴，行为更符合直觉。
 */
import { onBeforeUnmount, onMounted } from 'vue'
import { imageApi } from '@/api'
import { useIsActive } from '@/composables/useIsActive'
import { errorMessage } from '@/types/error'

export interface UsePasteImageOptions {
  /** 导入成功：回调新图片 id（可能一次多张） */
  onImported: (ids: string[]) => void
  /**
   * 还能再放几张（返回剩余额度）。
   *
   * 返回 `0` 时不处理粘贴并提示「已达上限」。不传则不限。
   */
  remaining?: () => number
  /** 是否启用（如正在生成中时禁用）。默认一直启用 */
  enabled?: () => boolean
}

/** 从粘贴事件里取出图片文件（没有则返回空数组） */
function imageFilesFrom(e: ClipboardEvent): File[] {
  const dt = e.clipboardData
  if (!dt) return []

  const out: File[] = []
  // `files` 在部分 WebView 上比 `items` 更可靠，两个都看一遍
  for (const f of Array.from(dt.files ?? [])) {
    if (f.type.startsWith('image/')) out.push(f)
  }
  if (out.length > 0) return out

  for (const item of Array.from(dt.items ?? [])) {
    if (item.kind !== 'file') continue
    if (!item.type.startsWith('image/')) continue
    const f = item.getAsFile()
    if (f) out.push(f)
  }
  return out
}

/** `File` → data URL */
function readAsDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const fr = new FileReader()
    fr.onload = () => resolve(String(fr.result))
    fr.onerror = () => reject(fr.error ?? new Error('读取剪贴板图片失败'))
    fr.readAsDataURL(file)
  })
}

export function usePasteImage(opts: UsePasteImageOptions) {
  /** 正在导入（防重入：一次粘贴可能触发多个事件） */
  let busy = false

  /**
   * 🔴 只有**当前可见的那一页**才处理粘贴。
   *
   * `App.vue` 用 `keep-alive` 缓存视图，所以「新建公式」与「模型测试」
   * 会**同时活着**，两个 `ImageAttachRow` 都挂着 `window` 的 paste 监听。
   * 没有这道判断时，粘一张图会被**两页同时收下**（附件状态串用）。
   */
  const active = useIsActive()

  async function onPaste(e: ClipboardEvent): Promise<void> {
    if (!active.value) return
    if (opts.enabled && !opts.enabled()) return

    const files = imageFilesFrom(e)
    // 🔴 没有图片就**什么都不做** —— 让文本照常粘进输入框
    if (files.length === 0) return

    // 有图片：吃掉这次粘贴，避免同时往输入框塞乱码
    e.preventDefault()
    if (busy) return

    const remain = opts.remaining ? opts.remaining() : Number.POSITIVE_INFINITY
    if (remain <= 0) {
      ElMessage.warning('图片数量已达上限')
      return
    }

    const take = files.slice(0, remain)
    if (take.length < files.length) {
      ElMessage.warning(`已达上限，只导入前 ${take.length} 张`)
    }

    busy = true
    try {
      const ids: string[] = []
      for (const f of take) {
        const dataUrl = await readAsDataUrl(f)
        ids.push(await imageApi.imageSaveFromBytes(dataUrl))
      }
      opts.onImported(ids)
      ElMessage.success(ids.length > 1 ? `已粘贴 ${ids.length} 张图片` : '已粘贴图片')
    } catch (err) {
      ElMessage.error(`粘贴图片失败：${errorMessage(err as never)}`)
    } finally {
      busy = false
    }
  }

  onMounted(() => window.addEventListener('paste', onPaste))
  onBeforeUnmount(() => window.removeEventListener('paste', onPaste))
}
