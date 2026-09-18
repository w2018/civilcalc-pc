<script setup lang="ts">
/**
 * 标题栏 —— 见 docs/05-项目开发方案.md §2.1.1。
 *
 * 三段：面包屑（左） + 全局搜索框（中） + 主题切换（右）。
 *
 * ## 搜索框为什么放在标题栏
 *
 * 搜索是最高频入口（「找公式」）。放标题栏可以让用户在**任何页面**
 * 直接搜，不用先跳回搜索页 —— 少一次跳转。
 *
 * ## 防抖 200ms + 请求序号
 *
 * `el-autocomplete` 自带 debounce（这里设 200ms），但仍会**并发**发出请求。
 * 用户快速打字时，「梁」的响应可能晚于「梁正」到达，导致下拉里显示旧结果。
 * 因此用 `seq` 序号丢弃过期响应 —— 与 `useEval` 是同一套防竞态思路。
 */
import { computed, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { searchApi } from '@/api'
import { useHotkeys } from '@/composables/useHotkeys'
import { nextTheme, themeIcon, themeLabel } from '@/composables/useTheme'
import { ROUTE_TITLES } from '@/router/nav'
import { useUiStore } from '@/stores/ui'
import SourceBadge from '@/components/common/SourceBadge.vue'
import type { SearchSuggestion } from '@/types/domain'

const route = useRoute()
const router = useRouter()
const ui = useUiStore()

/** 搜索框实例（`Ctrl+K` 聚焦用） */
const searchRef = ref<{ focus: () => void } | null>(null)

/**
 * `Ctrl+K` —— 聚焦全局搜索。
 *
 * 放在标题栏而不是各页面：搜索是全局入口，任何页面都该能一键唤起。
 * 具体为什么不做全局注册表见 `composables/useHotkeys.ts`。
 */
useHotkeys({
  'ctrl+k': () => searchRef.value?.focus(),
})

/** 当前主题的图标与中文名（标题栏按钮用） */
const themeIconText = computed(() => themeIcon(ui.themeMode))
const themeName = computed(() => themeLabel(ui.themeMode))

/** 轮转到下一个主题：跟随系统 → 亮 → 暗 → 跟随系统 */
function cycleTheme(): void {
  void ui.setTheme(nextTheme(ui.themeMode))
}

/** 面包屑文案：`ROUTE_TITLES` 兜底，视图可通过 `meta.title` 覆盖 */
const pageTitle = computed(() => {
  const fromMeta = route.meta.title
  if (typeof fromMeta === 'string' && fromMeta) return fromMeta
  const name = typeof route.name === 'string' ? route.name : ''
  return ROUTE_TITLES[name] ?? ''
})

// ---------------------------------------------------------------- 搜索

/** 下拉里的一项：真实建议，或「用 AI 解析」兜底项 */
type SuggestionItem = SearchSuggestion | { aiFallback: true; query: string }

function isAiFallback(x: SuggestionItem): x is { aiFallback: true; query: string } {
  return 'aiFallback' in x
}

/** 请求序号：用于丢弃过期响应 */
let seq = 0

const suggestValue = ref('')

/**
 * `el-autocomplete` 的取数回调。
 *
 * 未命中时**不返回空数组**，而是塞一个兜底项 —— 让「用 AI 解析？」
 * 也能被键盘选中，否则用户打完字发现没结果，还要去点鼠标。
 */
async function fetchSuggestions(
  query: string,
  cb: (items: SuggestionItem[]) => void,
): Promise<void> {
  const q = query.trim()
  if (!q) {
    cb([])
    return
  }

  const mine = ++seq
  try {
    const list = await searchApi.searchSuggest(q, 8)
    if (mine !== seq) return // 已有更新的请求发出，丢弃本次结果
    cb(list.length > 0 ? list : [{ aiFallback: true, query: q }])
  } catch (e) {
    if (mine !== seq) return
    console.warn('[TopBar] 搜索建议失败', e)
    cb([])
  }
}

/**
 * 选中一项：真实公式 → 跳工作台；兜底项 → 跳描述页并带上关键词。
 *
 * ⚠️ 参数类型**必须**是 `Record<string, any>` —— 这是 `el-autocomplete`
 * 的 `select` 事件签名。写窄类型（`SuggestionItem`）会因**参数逆变**报
 * TS2322：组件库传进来的是宽类型，回调不能要求更窄。
 * 因此这里收下宽类型，再显式断言回 `SuggestionItem`。
 */
function onSelect(item: Record<string, any>): void {
  const picked = item as SuggestionItem
  if (isAiFallback(picked)) {
    router.push({ name: 'query', query: { q: picked.query } })
  } else {
    router.push({ name: 'formula', params: { id: picked.id } })
  }
  suggestValue.value = ''
}
</script>

<template>
  <header class="topbar">
    <h1 v-if="pageTitle" class="topbar__title">{{ pageTitle }}</h1>

    <div class="topbar__spacer" />

    <el-autocomplete
      ref="searchRef"
      v-model="suggestValue"
      class="topbar__search"
      placeholder="搜索公式（中文 / 拼音 / 首字母）"
      :fetch-suggestions="fetchSuggestions"
      :debounce="200"
      :trigger-on-focus="false"
      clearable
      @select="onSelect"
    >
      <template #prefix>
        <span aria-hidden="true">🔍</span>
      </template>
      <template #default="{ item }">
        <div v-if="isAiFallback(item)" class="sug sug--ai">
          <span class="sug__icon" aria-hidden="true">💬</span>
          <span class="sug__name">用 AI 解析「{{ item.query }}」</span>
        </div>
        <div v-else class="sug">
          <span class="sug__name">{{ item.resultName }}</span>
          <span class="sug__domain">{{ item.domain }}</span>
          <SourceBadge :kind="item.sourceKind" compact />
        </div>
      </template>
    </el-autocomplete>

    <el-tooltip :content="`主题：${themeName}`" placement="bottom">
      <button
        class="topbar__theme"
        type="button"
        :aria-label="`切换主题（当前 ${themeName}）`"
        @click="cycleTheme()"
      >
        <span aria-hidden="true">{{ themeIconText }}</span>
      </button>
    </el-tooltip>
  </header>
</template>

<style scoped>
.topbar {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
  height: var(--row-h);
  flex-shrink: 0;
  padding: 0 var(--sp-5);
  background: var(--c-surface);
  border-bottom: var(--hairline) solid var(--c-divider);
}

.topbar__title {
  margin: 0;
  font-size: var(--f-size-lg);
  font-weight: 600;
  color: var(--c-text);
  white-space: nowrap;
}

.topbar__spacer {
  flex: 1;
}

.topbar__search {
  width: 320px;
}

.topbar__theme {
  flex-shrink: 0;
  width: 32px;
  height: 32px;
  border: none;
  border-radius: var(--r-btn);
  background: transparent;
  font-size: var(--f-size-lg);
  line-height: 1;
  cursor: pointer;
}

.topbar__theme:hover {
  background: var(--c-surface-hover);
}

/* 下拉项 */
.sug {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  overflow: hidden;
}

.sug__name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sug__domain {
  flex-shrink: 0;
  font-size: var(--f-size-xs);
  color: var(--c-text-3);
}

.sug__icon {
  flex-shrink: 0;
}

.sug--ai .sug__name {
  color: var(--c-primary);
}
</style>
