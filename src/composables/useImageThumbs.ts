/**
 * 图片缩略图懒加载（相册九宫格 / 图片缓存九宫格共用）。
 *
 * ## 为什么必须懒加载
 *
 * `image_load_data_url` 每次都要读盘 + base64 编码。图片缓存可能有几百张，
 * 一次性全取会在打开页面时卡住好几秒。
 * 这里用 `IntersectionObserver`：格子进入视口才去取。
 *
 * ## 为什么缓存不放 Pinia
 *
 * 缓存的生命周期与**组件**绑定（离开页面就该释放，否则几百张图
 * 的 base64 会一直挂在内存里）。放模块级单例反而会漏。
 *
 * ## 用法
 *
 * ```html
 * <img :ref="(el) => bindLazy(el as Element | null, it.id)" :src="thumbs[it.id]" />
 * ```
 *
 * 注意 `:ref` 在**每个分支**上都要挂 —— 只挂在 `v-if` 为真的那个分支上，
 * 未加载时就没有元素可观察，图片永远不会开始加载。
 */
import { onBeforeUnmount, ref } from 'vue'
import { imageApi } from '@/api'

export function useImageThumbs() {
  /** id → data URL（未加载的键不存在） */
  const thumbs = ref<Record<string, string>>({})

  let observer: IntersectionObserver | null = null

  function ensureObserver(): IntersectionObserver {
    if (observer) return observer
    observer = new IntersectionObserver((entries) => {
      for (const en of entries) {
        if (!en.isIntersecting) continue
        const target = en.target as HTMLElement
        const id = target.dataset.imgId
        if (!id) continue
        observer?.unobserve(target)
        void load(id)
      }
    })
    return observer
  }

  /** 绑到格子的 `:ref` 上 */
  function bindLazy(el: Element | null, id: string): void {
    if (!el) return
    if (thumbs.value[id]) return
    const target = el as HTMLElement
    target.dataset.imgId = id
    ensureObserver().observe(target)
  }

  /** 取一张（已取过则跳过） */
  async function load(id: string): Promise<void> {
    if (thumbs.value[id]) return
    try {
      const url = await imageApi.imageLoadDataUrl(id)
      if (url) thumbs.value = { ...thumbs.value, [id]: url }
    } catch {
      // 单张失败不影响整页（图片可能刚被删）
    }
  }

  /**
   * 丢掉不在 `keep` 里的缓存。
   *
   * 删除图片后必须调一次 —— 否则「删了还能看到」。
   */
  function prune(keep: string[]): void {
    const next: Record<string, string> = {}
    for (const id of keep) if (thumbs.value[id]) next[id] = thumbs.value[id]
    thumbs.value = next
  }

  /** 主动加载一批（已缓存或已失效的跳过） */
  async function ensure(ids: string[]): Promise<void> {
    for (const id of ids) {
      if (!thumbs.value[id]) await load(id)
    }
  }

  /** 取某张的 URL（没有返回 `null`） */
  function urlOf(id: string): string | null {
    return thumbs.value[id] ?? null
  }

  function dispose(): void {
    observer?.disconnect()
    observer = null
  }

  onBeforeUnmount(dispose)

  return { thumbs, bindLazy, load, ensure, prune, urlOf, dispose }
}
