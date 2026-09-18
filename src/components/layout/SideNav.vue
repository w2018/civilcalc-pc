<script setup lang="ts">
/**
 * 侧边导航 —— 见 docs/05-项目开发方案.md §2.1.1。
 *
 * - 12 个入口分 4 组（分组依据见 `@/router/nav.ts`）
 * - 可折叠：折叠后只剩图标，用 tooltip 补名称
 * - 高亮用**精确匹配**（`route.path === item.to`）：本应用的路由全是平级，
 *   没有嵌套关系，前缀匹配会让 `/formula/xxx` 之类的子路径误高亮
 */
import { computed } from 'vue'
import { useRoute } from 'vue-router'
import { NAV_GROUPS, type NavItem } from '@/router/nav'
import AppLogo from '@/components/common/AppLogo.vue'
import NavIcon from '@/components/common/NavIcon.vue'
import { useUiStore } from '@/stores/ui'
import { useFormulaStore } from '@/stores/formula'
import { APP_VERSION_TAG } from '@/utils/version'

const route = useRoute()
const ui = useUiStore()
const formula = useFormulaStore()

const collapsed = computed(() => ui.navCollapsed)

/** 侧栏宽度（CSS v-bind 用；显式算好避免在样式里做字符串拼接） */
const navWidthPx = computed(() => `${ui.navWidth}px`)

/**
 * 当前路径是否命中该导航项。
 *
 * - 默认**精确匹配**：本应用路由全是平级，前缀匹配会让
 *   `/formula/xxx` 之类的子路径误高亮别的项
 * - 标了 `matchPrefix` 的项（公式计算：入口是 `/formula`、
 *   实际工作在 `/formula/<id>`）用前缀匹配
 */
function isActive(item: NavItem): boolean {
  if (item.matchPrefix) {
    return route.path === item.to || route.path.startsWith(`${item.to}/`)
  }
  return route.path === item.to
}

/**
 * 导航项实际跳转的目标。
 *
 * ## 「公式计算」要回到**上一次在算的那条公式**
 *
 * 这一项的 `to` 是 `/formula`（无 id）—— 那是「还没选公式」时的空态引导。
 * 但用户上一次很可能正在算某条公式（`/formula/<id>`）。
 * 这时点侧栏如果跳到 `/formula`，看到的就是一块「请从收藏/历史里选一条」
 * 的空态 —— 用户会以为**切走再切回把状态弄丢了**（组件其实还活着，
 * 只是被路由带到了空态页）。
 *
 * 所以有「上次用的公式」时就回到它；没有才回落到空态入口。
 *
 * ## 🔴 连 `historyId` 一起带上
 *
 * 用户可能是从「历史」点某条记录进来的（`/formula/<id>?historyId=<n>`）。
 * 只带 id 会落到**同一条公式的干净版本**上 —— 而路由守卫现在把
 * 「同一公式换历史」也当作一次切换，于是点侧栏反而会弹「要切换公式吗？」，
 * 用户明明只是想回到刚才那一条。
 *
 * 带上 `historyId` 后目标与当前位置**完全一致**，守卫直接放行，
 * 工作台内容也不会被重建。
 */
function targetOf(item: NavItem): string {
  if (item.name === 'formula' && formula.lastWorkspaceId) {
    const base = `${item.to}/${formula.lastWorkspaceId}`
    return formula.lastWorkspaceHistoryId
      ? `${base}?historyId=${formula.lastWorkspaceHistoryId}`
      : base
  }
  return item.to
}
</script>

<template>
  <aside class="nav" :class="{ 'nav--collapsed': collapsed }" :style="{ width: navWidthPx }">
    <div class="nav__brand">
      <!-- 品牌标识（需求 8）：与「设置」「关于」页同一套内联 SVG，
           不再用 emoji —— emoji 的字体/配色随系统变，和整站扁平风不是一套语言 -->
      <AppLogo class="nav__logo" :size="26" />
      <span v-if="!collapsed" class="nav__title">AI全能计算器</span>
    </div>

    <nav class="nav__groups" aria-label="主导航">
      <template v-for="(group, gi) in NAV_GROUPS" :key="group.label">
        <hr v-if="gi > 0" class="nav__divider" />
        <ul class="nav__group">
          <li v-for="item in group.items" :key="item.name">
            <el-tooltip
              :content="item.label"
              placement="right"
              :disabled="!collapsed"
              :show-after="300"
            >
              <RouterLink
                class="nav__item"
                :class="{ 'nav__item--active': isActive(item) }"
                :to="targetOf(item)"
              >
                <NavIcon class="nav__icon" :name="item.icon" :size="20" />
                <span v-if="!collapsed" class="nav__label">{{ item.label }}</span>
              </RouterLink>
            </el-tooltip>
          </li>
        </ul>
      </template>
    </nav>

    <div class="nav__footer">
      <button
        class="nav__toggle"
        type="button"
        :aria-label="collapsed ? '展开侧边栏' : '折叠侧边栏'"
        :aria-expanded="!collapsed"
        @click="ui.toggleNav()"
      >
        <span aria-hidden="true">{{ collapsed ? '»' : '«' }}</span>
        <span v-if="!collapsed" class="nav__label">折叠</span>
      </button>
      <div v-if="!collapsed" class="nav__version">{{ APP_VERSION_TAG }}</div>
    </div>
  </aside>
</template>

<style scoped>
.nav {
  display: flex;
  flex-direction: column;
  /* 宽度由模板的 :style 绑定（CSS v-bind 在 vue-tsc 下会被误报未使用变量） */
  flex-shrink: 0;
  background: var(--c-surface);
  border-right: var(--hairline) solid var(--c-divider);
  transition: width 0.16s ease;
}

.nav__brand {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  height: var(--row-h);
  flex-shrink: 0;
  padding: 0 var(--sp-4);
  border-bottom: var(--hairline) solid var(--c-divider);
  overflow: hidden;
}

.nav__logo {
  flex-shrink: 0;
}

.nav__title {
  font-size: var(--f-size-lg);
  font-weight: 600;
  color: var(--c-text);
  white-space: nowrap;
}

.nav__groups {
  flex: 1;
  overflow-y: auto;
  padding: var(--sp-2) 0;
}

.nav__group {
  list-style: none;
  margin: 0;
  padding: 0;
}

.nav__divider {
  border: none;
  border-top: var(--hairline) solid var(--c-divider);
  margin: var(--sp-2) var(--sp-3);
}

.nav__item {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
  height: 40px;
  margin: 0 var(--sp-2);
  padding: 0 var(--sp-3);
  border-radius: var(--r-btn);
  color: var(--c-text);
  text-decoration: none;
  white-space: nowrap;
  overflow: hidden;
}

.nav__item:hover {
  background: var(--c-surface-hover);
}

.nav__item--active {
  background: var(--c-primary-light);
  color: var(--c-primary);
  font-weight: 600;
}

/* 图标槽：SVG 自带 20×20，这里只管对齐与居中 */
.nav__icon {
  flex-shrink: 0;
  width: 20px;
  height: 20px;
}

.nav__label {
  font-size: var(--f-size-base);
}

.nav__footer {
  flex-shrink: 0;
  padding: var(--sp-2);
  border-top: var(--hairline) solid var(--c-divider);
}

.nav__toggle {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
  width: 100%;
  height: 36px;
  padding: 0 var(--sp-3);
  border: none;
  border-radius: var(--r-btn);
  background: transparent;
  color: var(--c-text-3);
  font-family: inherit;
  font-size: var(--f-size-sm);
  cursor: pointer;
  white-space: nowrap;
  overflow: hidden;
}

.nav__toggle:hover {
  background: var(--c-surface-hover);
  color: var(--c-text);
}

.nav__version {
  padding: var(--sp-1) var(--sp-3) var(--sp-2);
  font-size: var(--f-size-xs);
  color: var(--c-text-3);
}
</style>
