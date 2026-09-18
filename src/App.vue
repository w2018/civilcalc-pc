<script setup lang="ts">
/**
 * AppShell —— 左侧导航 + 右侧主工作区。
 *
 * 见 docs/05-项目开发方案.md §2.1.1。
 *
 * ## 外观状态在这里拉取
 *
 * 主题已在 `main.ts` 里用默认值应用过一次（避免首屏闪色），
 * 这里 `onMounted` 再向后端要一次真实值做纠正。
 * **不阻塞挂载** —— 外观读不到不该让应用打不开。
 *
 * ## 🔴 为什么必须 `keep-alive`（切标签不丢状态、不中断异步操作）
 *
 * 路由用的是 `() => import(...)` 懒加载。**不加 `keep-alive` 时，
 * 每次切标签都会销毁旧组件再新建一个** —— 于是：
 *
 * - 「新建公式」页打了一半的需求文字、已选的附图全没了
 * - **AI 正在流式生成时切走 → 组件被销毁、事件订阅被解绑、进度全丢**
 *   （异步任务在后端还在跑，但界面再也收不到结果）
 * - 「公式库」的搜索词、筛选、滚动位置回到初始
 * - 「公式计算」页的思考过程、参数、结果全部清空
 *
 * 桌面应用里这很反直觉（用户认为切走再切回来东西还在），
 * 所以**全部视图一律缓存**。
 *
 * ### 缓存键一律用**路由名**
 *
 * 用 `fullPath` 当键会让 `/formula/a` 与 `/formula/b` 各占一个实例，
 * 而公式工作台的数据在**全局 Pinia 单例**里 —— 两个实例会争同一份 state
 * 互相覆盖（这是早期版本踩过的坑）。改用路由名后**每个页面只有一份实例**，
 * 带 id 的页面自己 `watch` 参数变化后重新加载
 * （`FormulaView` 监听 `[formulaId, historyId]`、`VersionView` 监听 `formulaId`）。
 *
 * 同理不能用 `fullPath`：`/query?q=xxx` 这类带 query 的跳转会新开实例，
 * 反而把已经打了一半的状态丢了。
 *
 * ### 缓存了就要管**数据新鲜度**
 *
 * 列表类页面被缓存后不会自动重挂载，新增数据时它们仍是旧快照。
 * 这些页面各自用 `onActivated` 重新拉取（见各视图内的说明）。
 */
import { onMounted } from 'vue'
import type { RouteLocationNormalizedLoaded } from 'vue-router'
import SideNav from '@/components/layout/SideNav.vue'
import TopBar from '@/components/layout/TopBar.vue'
import { useUiStore } from '@/stores/ui'

const ui = useUiStore()

/**
 * 缓存键：**一律用路由名**（见上方说明）。
 *
 * 路由名一定存在（`router/index.ts` 里每条都写了 `name`），
 * 兜底用 `path` 只是防御性写法。
 */
function cacheKey(r: RouteLocationNormalizedLoaded): string {
  return String(r.name ?? r.path)
}

onMounted(() => {
  void ui.loadAppearance()
})
</script>

<template>
  <div class="shell">
    <SideNav />

    <div class="shell__main">
      <TopBar />
      <main class="shell__content">
        <RouterView v-slot="{ Component, route: r }">
          <!-- `:max` 必须 ≥ 路由总数（12），否则最先进入的页面会被挤出去 ——
               表现为「切回去状态没了」，正是这次要修的问题 -->
          <KeepAlive :max="16">
            <component :is="Component" :key="cacheKey(r)" />
          </KeepAlive>
        </RouterView>
      </main>
    </div>
  </div>
</template>

<style scoped>
.shell {
  display: flex;
  height: 100%;
  /* 窗口缩放时子项必须可收缩：flex 项默认 `min-width: auto`，
     内容比容器宽时会把容器撑破，表现为「窗口缩小了但内容不跟着缩」 */
  min-width: 0;
}

.shell__main {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-width: 0;
}

.shell__content {
  flex: 1;
  min-width: 0;
  overflow: auto;
  padding: var(--sp-5);
}

/* 窄窗口（<900px）收紧内边距，给内容让出空间 */
@media (max-width: 900px) {
  .shell__content {
    padding: var(--sp-3);
  }
}
</style>
