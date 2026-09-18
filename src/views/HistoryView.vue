<script setup lang="ts">
/**
 * 历史记录 —— 可回填复现 + 思考过程回溯。
 *
 * 见 docs/05-项目开发方案.md §2.1.2。
 *
 * ## 回填走 `formulaId` + `historyId`，**不传大 JSON**
 *
 * 一次计算的 `inputsJson` 可能几十 KB。把它塞进路由 query 会让
 * URL 长得没法看、也没法分享。所以路由只带 id：
 *
 * ```
 * /formula/usr:1?historyId=42
 * ```
 *
 * 工作台拿到 `historyId` 后调 `history_get` 取回输入值预填。
 *
 * ## 公式名用**快照**里的
 *
 * 公式可能已被改名或删除。历史列表显示快照名，才反映「那一次算的是什么」。
 * 公式已删时仍可点进历史看当时的输入与结果（工作台会提示公式不存在）。
 *
 * ## 清空必须二次确认
 *
 * `history_clear` 要求 `confirm: true`；这里用 `ElMessageBox` 弹确认框，
 * 并**明确说明不可撤销**。
 */
import { computed, onActivated, ref } from 'vue'
import { useRouter } from 'vue-router'
import { historyApi } from '@/api'
import { errorMessage } from '@/types/error'
import { formatTime } from '@/utils/format'
import { formatNumber } from '@/utils/format'
import {
  historyDisplayName,
  parseHistoryResult,
  type HistoryEntry,
} from '@/types/domain'

const router = useRouter()

const entries = ref<HistoryEntry[]>([])
const loading = ref(true)
const error = ref('')

/** 每页条数（后端上限 500） */
const PAGE_SIZE = 50
const hasMore = ref(false)

async function load(offset = 0): Promise<void> {
  loading.value = true
  error.value = ''
  try {
    const list = await historyApi.historyList(undefined, PAGE_SIZE, offset)
    entries.value = offset === 0 ? list : [...entries.value, ...list]
    hasMore.value = list.length === PAGE_SIZE
  } catch (e) {
    error.value = errorMessage(e as never)
  } finally {
    loading.value = false
  }
}


/**
 * 被 `keep-alive` 缓存后**不会重新挂载**，切回来时数据仍是旧快照。
 * 所以每次激活都重新拉一次 —— 否则在别处新增/删除了记录，
 * 这里要等应用重启才看得到。
 */
onActivated(() => {
  void load(0)
})

/** 每条历史的主结果文案（无结果时留空） */
function primaryText(e: HistoryEntry): string {
  const r = parseHistoryResult(e)
  if (!r) return ''
  return formatNumber(r.primary)
}

/**
 * 「只记录生成、还没算过」的条目（需求 7）。
 *
 * AI 生成 / 微调出新公式时会立刻记一条**无输入无结果**的历史 ——
 * 这类条目右侧没有数值，不给标记的话看起来像坏数据。
 */
function isParseOnly(e: HistoryEntry): boolean {
  return parseHistoryResult(e) === null
}

/** 参数摘要（`a=1.5，b=2`）；最多显示 4 个，避免撑破行 */
function paramsSummary(e: HistoryEntry): string {
  const raw = e.inputsJson?.trim()
  if (!raw || raw === '{}') return ''
  try {
    const inputs = JSON.parse(raw) as Record<string, number>
    const keys = Object.keys(inputs)
    if (keys.length === 0) return ''
    const head = keys
      .slice(0, 4)
      .map((k) => `${k}=${formatNumber(inputs[k])}`)
      .join('，')
    return keys.length > 4 ? `${head} …` : head
  } catch {
    return ''
  }
}

/** 回填到工作台 */
function restore(e: HistoryEntry): void {
  router.push({
    name: 'formula',
    params: { id: e.formulaId },
    query: { historyId: String(e.id) },
  })
}

/** 删除单条 */
async function remove(e: HistoryEntry): Promise<void> {
  try {
    await ElMessageBox.confirm(`删除「${historyDisplayName(e)}」的这条历史？`, '删除历史', {
      confirmButtonText: '删除',
      cancelButtonText: '取消',
      type: 'warning',
    })
  } catch {
    return // 用户取消
  }

  try {
    await historyApi.historyDelete(e.id)
    entries.value = entries.value.filter((x) => x.id !== e.id)
    ElMessage.success('已删除')
  } catch (err) {
    ElMessage.error(errorMessage(err as never))
  }
}

/** 清空全部（必须 confirm=true） */
async function clearAll(): Promise<void> {
  try {
    await ElMessageBox.confirm(
      '将删除全部计算历史，此操作不可撤销。公式本身不会被删除。',
      '清空历史',
      {
        confirmButtonText: '清空',
        cancelButtonText: '取消',
        type: 'warning',
      },
    )
  } catch {
    return
  }

  try {
    const n = await historyApi.historyClear(true)
    entries.value = []
    ElMessage.success(`已清空 ${n} 条历史`)
  } catch (err) {
    ElMessage.error(errorMessage(err as never))
  }
}

const isEmpty = computed(() => !loading.value && entries.value.length === 0)
</script>

<template>
  <div class="view">
    <div class="toolbar">
      <span class="toolbar__count">{{ entries.length }} 条</span>
      <button v-if="entries.length > 0" class="link link--danger" type="button" @click="clearAll()">
        清空历史
      </button>
    </div>

    <p v-if="error" class="state state--error">{{ error }}</p>

    <el-skeleton v-else-if="loading && entries.length === 0" :rows="5" animated />

    <div v-else-if="isEmpty" class="state">
      <p class="state__text">还没有计算历史</p>
      <p class="state__hint">在公式工作台点「记录本次计算」即可留下一条</p>
    </div>

    <div v-else class="panel">
      <div
        v-for="e in entries"
        :key="e.id"
        class="item"
        role="button"
        tabindex="0"
        @click="restore(e)"
        @keydown.enter.prevent="restore(e)"
      >
        <div class="item__main">
          <div class="item__title">{{ historyDisplayName(e) }}</div>
          <div class="item__sub">
            <span class="item__time">{{ formatTime(e.createdAt) }}</span>
            <span v-if="paramsSummary(e)" class="item__params">{{ paramsSummary(e) }}</span>
          </div>
        </div>

        <span v-if="primaryText(e)" class="item__value">{{ primaryText(e) }}</span>
        <span v-else-if="isParseOnly(e)" class="item__tag">仅生成</span>

        <button
          class="item__del"
          type="button"
          aria-label="删除这条历史"
          @click.stop="remove(e)"
        >
          ✕
        </button>
      </div>

      <div v-if="hasMore" class="more">
        <button class="link" type="button" @click="load(entries.length)">加载更多</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.view {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  max-width: none;
}

.toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.toolbar__count {
  font-size: var(--f-size-sm);
  color: var(--c-text-3);
}

.panel {
  border-radius: var(--r-card);
  background: var(--c-surface);
  overflow: hidden;
}

.item {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
  min-height: var(--row-h);
  padding: var(--sp-2) var(--sp-4);
  cursor: pointer;
}

.item:hover {
  background: var(--c-surface-hover);
}

.item:not(:last-child) {
  border-bottom: var(--hairline) solid var(--c-divider);
}

.item__main {
  flex: 1;
  min-width: 0;
}

.item__title {
  font-size: var(--f-size-base);
  color: var(--c-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.item__sub {
  display: flex;
  gap: var(--sp-2);
  margin-top: 2px;
  font-size: var(--f-size-xs);
  color: var(--c-text-3);
  overflow: hidden;
}

.item__params {
  font-family: var(--f-mono);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 「仅生成」标记：这类条目没有计算结果，标一下免得像坏数据 */
.item__tag {
  flex-shrink: 0;
  padding: 1px var(--sp-2);
  border-radius: var(--r-pill);
  background: var(--c-surface-2);
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
  line-height: 18px;
}

.item__value {
  flex-shrink: 0;
  font-family: var(--f-mono);
  font-size: var(--f-size-base);
  font-weight: 600;
  color: var(--c-text);
}

.item__del {
  flex-shrink: 0;
  width: 26px;
  height: 26px;
  border: none;
  border-radius: var(--r-btn);
  background: transparent;
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
  cursor: pointer;
}

.item__del:hover {
  background: var(--c-surface-hover);
  color: var(--c-danger);
}

.more {
  padding: var(--sp-3);
  text-align: center;
}

.state {
  margin: 0;
  padding: var(--sp-8) var(--sp-4);
  border-radius: var(--r-card);
  background: var(--c-surface);
  text-align: center;
  font-size: var(--f-size-sm);
  color: var(--c-text-3);
}

.state--error {
  color: var(--c-danger);
}

.state__text {
  margin: 0;
}

.state__hint {
  margin: var(--sp-2) 0 0;
  font-size: var(--f-size-xs);
}

.link {
  border: none;
  background: transparent;
  padding: 0;
  color: var(--c-primary);
  font-family: inherit;
  font-size: var(--f-size-sm);
  cursor: pointer;
}

.link:hover {
  text-decoration: underline;
}

.link--danger {
  color: var(--c-danger);
}
</style>
