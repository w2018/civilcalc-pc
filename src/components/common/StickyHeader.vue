<script setup lang="ts">
/**
 * 折叠栏吸附置顶（P3-15）。
 *
 * 见 docs/05-项目开发方案.md §2.3.2 的 `StickyHeader` 规范与
 * docs/02-PC端迁移与开发规划.md 的「偏移量测试逻辑」。
 *
 * ## 为什么网页端比源项目简单
 *
 * 源项目（Compose）**没有原生 sticky**，所以 `StickyHeader.kt` 要手算
 * 「滚动量 − 标题位置」做 `graphicsLayer` 平移，还要 `StickyHeaderOffsetTest`
 * 验证偏移量 = 滚动容器顶部 inset。
 *
 * **网页端有原生 `position: sticky`** —— 这里只是个薄包装：
 *
 * - `top` = 标题要钉在离滚动视口顶部的偏移（对齐源项目「偏移量」概念）
 * - 进不进吸附由**父区块高度**决定（无需 JS）：展开的区块很高，
 *   标题在区块内滚到顶部就钉住；折叠的区块只剩标题一行，没空间钉，
 *   于是**自然随内容滚走** —— 这正是规范要的「展开吸附、折叠不吸附」。
 *
 * ## 用法
 *
 * 把标题行（或整条筛选栏）包进 `<StickyHeader>`，放在**可滚动区块**内、
 * 折叠内容**之前**。折叠状态由父组件控制（如 `v-show`），本组件不关心。
 *
 * ```html
 * <section>
 *   <StickyHeader :top="0"> …筛选栏/标题行… </StickyHeader>
 *   <div v-show="expanded"> …长内容… </div>
 * </section>
 * ```
 */
withDefaults(
  defineProps<{
    /** 钉在离滚动视口顶部的偏移（px）。等于源项目「偏移量」概念。 */
    top?: number
    /** 层级（避免被兄弟元素盖住） */
    zIndex?: number
    /**
     * 背景色（遮住下方滚动过来的内容）。
     * 默认用页面底色 `--c-bg`；若父区块是卡片，可传 `--c-surface`。
     */
    bg?: string
  }>(),
  { top: 0, zIndex: 10, bg: 'var(--c-bg)' },
)
</script>

<template>
  <div
    class="sticky"
    :style="{ top: top + 'px', zIndex, background: bg }"
  >
    <slot />
  </div>
</template>

<style scoped>
.sticky {
  position: sticky;
  /* 让吸附栏在内容上滚动时有实底，不穿透 */
  margin: 0 calc(-1 * var(--sp-5));
  padding: 0 var(--sp-5);
}
</style>
