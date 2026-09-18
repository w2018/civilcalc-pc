import { createRouter, createWebHashHistory, type RouteRecordRaw } from 'vue-router'
import { useFormulaStore } from '@/stores/formula'

/**
 * 路由表 —— 见 docs/05-项目开发方案.md §2.1.2。
 *
 * 用 **hash 模式**：桌面应用里避免 WebView 对 history 模式路径的处理差异。
 *
 * ## 🔴 为什么没有 `/search` 与 `/paste`（需求 10 / 11）
 *
 * - `/search` —— 与顶栏的全局搜索完全重复。顶栏那个在任何页面都能用
 *   （`Ctrl+K` 唤起），再留一个整页入口只是让人多跳一次。
 * - `/paste` —— 与 `/query` 是同一件事的两条路径（都是「把这段内容
 *   交给 AI 解析」），分成两个页面反而要求用户先判断手里的是哪一种。
 *
 * 两者都合并进 `/query`（「新建公式」）。这里**保留重定向**而不是直接删掉：
 * 应用内可能还有旧链接，而且开发期手动敲过的 hash 不至于变成空白页。
 *
 * ## 兜底路由
 *
 * 最后一条把未知路径收回 `/query`。没有它时，敲一个不存在的 hash
 * 会渲染出**一片空白**（没有任何视图匹配），用户只会以为应用坏了。
 */
const routes: RouteRecordRaw[] = [
  { path: '/', redirect: '/query' },

  // 已合并的旧入口 → 新入口（保留重定向，见上方说明）
  { path: '/search', redirect: '/query' },
  { path: '/paste', redirect: '/query' },

  // ---- 新建 ----
  { path: '/query', name: 'query', component: () => import('@/views/QueryView.vue') },

  // ---- 公式计算（核心页；需求 1 起同时作为左侧导航的固定入口）----
  //
  // `:id` 是**可选**的：导航栏那一项指向 `/formula`（无 id），
  // 此时页面显示「从收藏 / 历史 / 公式库选一条公式」的空态；
  // 从任何地方点某条公式进来则是 `/formula/<id>`。
  // 一个组件两种入口，避免再拆一个几乎一样的视图。
  {
    path: '/formula/:id?',
    name: 'formula',
    component: () => import('@/views/FormulaView.vue'),
    props: true,
  },
  {
    path: '/versions/:id',
    name: 'versions',
    component: () => import('@/views/VersionView.vue'),
    props: true,
  },

  // ---- 我的公式 ----
  { path: '/favorites', name: 'favorites', component: () => import('@/views/FavoritesView.vue') },
  { path: '/history', name: 'history', component: () => import('@/views/HistoryView.vue') },
  { path: '/library', name: 'library', component: () => import('@/views/LibraryView.vue') },
  { path: '/model-test', name: 'modelTest', component: () => import('@/views/ModelTestView.vue') },

  // ---- 数据与维护 ----
  { path: '/usage', name: 'usage', component: () => import('@/views/UsageView.vue') },
  { path: '/backup', name: 'backup', component: () => import('@/views/BackupView.vue') },
  { path: '/images', name: 'images', component: () => import('@/views/ImagesView.vue') },

  // ---- 系统 ----
  { path: '/settings', name: 'settings', component: () => import('@/views/SettingsView.vue') },
  { path: '/about', name: 'about', component: () => import('@/views/AboutView.vue') },

  // 兜底：未知路径回新建页
  { path: '/:pathMatch(.*)*', redirect: '/query' },
]

const router = createRouter({
  history: createWebHashHistory(),
  routes,
})

/**
 * 「会不会顶掉正在用的公式」——**唯一的**确认时机（需求 6）。
 *
 * ## 为什么放在这里，而不是某个页面的 `onBeforeRouteLeave`
 *
 * 需要拦的场景有**四个入口**：公式库 / 历史 / 收藏 / 刚生成的新公式。
 * 写在页面里只能管住自己那一个，四处各写一遍必然漏。
 *
 * ## 为什么需要 store 里的「占用标记」
 *
 * 离开「公式计算」页时 `store.schema` 已经被 `reset()` 清空了 ——
 * 守卫再读它永远是 `null`。所以判断依据是 `lastWorkspaceId` /
 * `lastWorkspaceName` / `lastWorkspaceHistoryId` 这几个
 * **刻意不被 `reset()` 清掉**的标记（见 `stores/formula.ts`）。
 *
 * ## 🔴 任何切换都要问（不再看「改没改过参数」）
 *
 * 曾经只在「改过参数」（`workspaceDirty`）时才问，理由是「只是看了一眼
 * 就换、没有东西会丢」。实测这个判断不可靠：换公式会把工作台内容
 * **整个换掉**，而「有没有东西会丢」在守卫这一层根本判断不准
 * （参数区可能只是被点开看了一眼、也可能是只改了没提交的草稿）。
 *
 * 用户明确要求「任何切换都要问」，所以现在只看**换没换**。
 *
 * ## 同一条公式的另一条历史也算「切换」
 *
 * 历史记录会带 `historyId` 查询参数回到那条快照。只比公式 id 会把
 * 「同一条公式换历史」当成没换 —— 而它同样会用快照整份重建参数，
 * 静默丢掉当前输入。所以这里连 `historyId` 一起比。
 *
 * 文案里如实说明「草稿会保留」——不编「不保存就丢失」的假警告。
 */
router.beforeEach(async (to) => {
  if (to.name !== 'formula') return true

  const nextId = String(to.params.id ?? '')
  // 导航栏那一项指向 `/formula`（无 id），显示的是空态引导，不覆盖任何东西
  if (!nextId) return true

  // 🔴 整个判断都包在 try 里：守卫抛异常会让**整个应用导航不动**
  // （表现为白屏）。这里是「锦上添花」的保护，任何一步出问题都应当
  // 直接放行，绝不能把用户挡在门外。
  let store
  try {
    store = useFormulaStore()
  } catch {
    // Pinia 还没装好（理论上不会 —— `main.ts` 里 `use(pinia)` 在 `use(router)` 之前）
    return true
  }

  const currentId = store.lastWorkspaceId
  // 从没打开过公式 → 放行
  if (!currentId) return true

  // 同一条公式的**同一条历史**才算「没换」；换了历史同样会替换工作台内容
  const nextHistoryId = String(to.query.historyId ?? '')
  if (currentId === nextId && store.lastWorkspaceHistoryId === nextHistoryId) return true

  try {
    await ElMessageBox.confirm(
      `工作台里正开着「${store.lastWorkspaceName || currentId}」。` +
        `切过去会离开这次计算（参数草稿会保留，下次打开还在）。`,
      '要切换公式吗？',
      { confirmButtonText: '切换', cancelButtonText: '留在这条', type: 'warning' },
    )
    return true
  } catch {
    return false
  }
})

export default router
