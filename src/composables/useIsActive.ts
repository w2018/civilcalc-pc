/**
 * 「我所在的页面现在是可见的那一页吗」—— 给挂在 `window` 上的全局监听用。
 *
 * ## 🔴 为什么必须有它（keep-alive 带来的新问题）
 *
 * `App.vue` 用 `<KeepAlive>` 缓存视图（切标签不丢状态）。代价是：
 * **切走的页面并没有卸载，只是被「停用」** —— 它的组件实例、以及挂在
 * `window` 上的事件监听**全都还活着**。
 *
 * 于是同一个事件会被多个页面同时处理：
 *
 * | 监听 | 症状 |
 * |---|---|
 * | `paste`（`usePasteImage`） | 在「模型测试」粘一张图，**「新建公式」也收到同一张**（附件状态串用） |
 * | `keydown`（`useHotkeys`） | 缓存页的快捷键在别的页面也会触发 |
 * | `keydown`（`ImageViewer` / `ThinkingBox` 全屏层） | 两页的查看器同时响应方向键 |
 *
 * ## 还有一个更隐蔽的：Teleport 不会被 keep-alive 收走
 *
 * `ImageViewer` 与 `ThinkingBox` 的全屏层是 `<Teleport to="body">`。
 * KeepAlive 停用组件时是把它的 **DOM 子树**移进隐藏容器 ——
 * 而 Teleport 出去的节点在 `body` 下，**不在那棵子树里**，
 * 于是「在 A 页打开查看器 → 切到 B 页」时，A 页的遮罩**会留在屏幕上**。
 * 所以这类覆盖层要同时判 `modelValue && active`。
 *
 * ## 用法
 *
 * ```ts
 * const active = useIsActive()
 * window.addEventListener('paste', (e) => { if (!active.value) return; … })
 * ```
 *
 * ## 不在 keep-alive 里的组件怎么办
 *
 * `onActivated` / `onDeactivated` 只在被 KeepAlive 管理的树里触发。
 * 不在其中的组件两者都不会跑，`active` 保持初始的 `true` ——
 * 这正是想要的（没被缓存就说明离开时是**卸载**，监听已经摘掉了）。
 */
import { onActivated, onDeactivated, ref, type Ref } from 'vue'

export function useIsActive(): Ref<boolean> {
  const active = ref(true)

  onActivated(() => {
    active.value = true
  })
  onDeactivated(() => {
    active.value = false
  })

  return active
}
