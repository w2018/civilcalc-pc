<script setup lang="ts">
/**
 * 公式库 —— 浏览全部公式，按领域归类 / 按来源筛选 / 仅看收藏。
 *
 * 见 docs/08-IPC契约.md 组 3（`formula_list`）与 docs/05 §2.1.2。
 *
 * ## 🔴 版式改成了「顶部工具条 + 卡片网格」（需求 9）
 *
 * 原来是「左侧竖排筛选栏 + 右侧卡片」的经典后台布局，问题在于：
 *
 * - 左侧栏占了 1/5 宽度却只放几个 chip，**空间利用率极低**
 * - 竖排 chip 在领域多时要滚很久，横向却有大片空白
 * - 卡片是「标题 + 代码 + 徽章」三层堆叠，**没有视觉层级**，
 *   扫一眼分不出哪个是名字、哪个是符号
 *
 * 现在：筛选全部横排到顶部（领域 chip 可横向滚动），
 * 内容区整宽给卡片网格。卡片改成「名字主标题 + 符号弱化副标 + 底部徽章」
 * 的层级，并用扁平描边 + hover 抬升，与全局的微信扁平风一致。
 *
 * ## 与顶栏搜索的分工
 *
 * 顶栏是**跨全部字段**的模糊搜索（含拼音），走 `search_suggest`；
 * 这里的关键词框只在**当前已加载的列表**里做即时过滤（不发 IPC），
 * 用于「我已经在公式库了，就想缩一下范围」。
 * 两者不重复：一个从无到有地找，一个在已有结果里收窄。
 *
 * ## 缓存与新鲜度
 *
 * 本页被 `keep-alive` 缓存（切标签不丢筛选状态），所以用 `onActivated`
 * 重新拉一次列表 —— 否则在别处新建了公式，切回来还看不到。
 * 筛选状态**故意保留**（那正是要保住的东西）。
 */
import { computed, onActivated, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { formulaList } from '@/api/formula'
import { favoritesList, toggleFavorite } from '@/api/favorite'
import type { FormulaSchema, SourceKind } from '@/types/domain'
import { sourceKindLabel } from '@/types/domain'
import SourceBadge from '@/components/common/SourceBadge.vue'
import { errorMessage } from '@/types/error'

const router = useRouter()

const all = ref<FormulaSchema[]>([])
const loading = ref(false)
const loadError = ref<string | null>(null)

/** 收藏集合（本地维护，toggle 后即时更新） */
const favSet = ref<Set<string>>(new Set())

/** 即时关键词过滤（本地，不发 IPC） */
const keyword = ref('')
const selectedDomain = ref<string | null>(null)
const selectedSource = ref<SourceKind | null>(null)
const favOnly = ref(false)

const SOURCES: SourceKind[] = ['STANDARD', 'AI', 'CUSTOM', 'DERIVED']

/** 领域集合（保持首次出现顺序） */
const domains = computed(() => {
  const seen = new Set<string>()
  const out: string[] = []
  for (const f of all.value) {
    if (f.domain && !seen.has(f.domain)) {
      seen.add(f.domain)
      out.push(f.domain)
    }
  }
  return out
})

/** 按筛选条件过滤后的列表 */
const filtered = computed(() => {
  const kw = keyword.value.trim().toLowerCase()
  return all.value.filter((f) => {
    if (selectedDomain.value && f.domain !== selectedDomain.value) return false
    if (selectedSource.value && f.source.kind !== selectedSource.value) return false
    if (favOnly.value && !favSet.value.has(f.id)) return false
    if (kw) {
      const hay = `${f.resultName} ${f.resultSymbol} ${f.domain} ${(f.tags ?? []).join(' ')}`
      if (!hay.toLowerCase().includes(kw)) return false
    }
    return true
  })
})

/** 是否处于「有筛选条件」状态（决定显示「清除筛选」） */
const hasFilter = computed(
  () =>
    keyword.value.trim().length > 0 ||
    selectedDomain.value !== null ||
    selectedSource.value !== null ||
    favOnly.value,
)

/** 按领域分组（用于「全部」视图） */
const grouped = computed(() => {
  const map = new Map<string, FormulaSchema[]>()
  for (const f of filtered.value) {
    const d = f.domain || '未分类'
    if (!map.has(d)) map.set(d, [])
    map.get(d)!.push(f)
  }
  return [...map.entries()]
})

async function load(): Promise<void> {
  loading.value = true
  loadError.value = null
  try {
    const [list, favs] = await Promise.all([formulaList(), favoritesList()])
    all.value = list
    favSet.value = new Set(favs.map((f: FormulaSchema) => f.id))
  } catch (e) {
    loadError.value = errorMessage(e as never)
  } finally {
    loading.value = false
  }
}

async function toggleFav(id: string, ev: Event): Promise<void> {
  ev.stopPropagation()
  try {
    const next = await toggleFavorite(id)
    if (next) favSet.value.add(id)
    else favSet.value.delete(id)
    // 触发响应性
    favSet.value = new Set(favSet.value)
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  }
}

function open(f: FormulaSchema): void {
  router.push({ name: 'formula', params: { id: f.id } })
}

function resetFilters(): void {
  keyword.value = ''
  selectedDomain.value = null
  selectedSource.value = null
  favOnly.value = false
}

onMounted(load)
// 被缓存后不会重新挂载 —— 回到本页时刷新列表（筛选状态保留）
onActivated(load)
</script>

<template>
  <section class="view">
    <!-- 顶部工具条：筛选全部横排，内容区整宽给卡片 -->
    <div class="toolbar">
      <div class="toolbar__row">
        <input
          v-model="keyword"
          class="search"
          type="search"
          placeholder="在当前列表里过滤名称 / 符号 / 领域…"
        />
        <span class="count">{{ filtered.length }} / {{ all.length }}</span>
        <button v-if="hasFilter" class="link" type="button" @click="resetFilters()">
          清除筛选
        </button>
      </div>

      <div class="toolbar__row toolbar__row--chips">
        <button
          class="chip"
          :class="{ 'chip--on': selectedDomain === null }"
          type="button"
          @click="selectedDomain = null"
        >
          全部领域
        </button>
        <button
          v-for="d in domains"
          :key="d"
          class="chip"
          :class="{ 'chip--on': selectedDomain === d }"
          type="button"
          @click="selectedDomain = d"
        >
          {{ d }}
        </button>

        <span class="toolbar__sep" aria-hidden="true" />

        <button
          v-for="s in SOURCES"
          :key="s"
          class="chip chip--src"
          :class="{ 'chip--on': selectedSource === s }"
          type="button"
          @click="selectedSource = selectedSource === s ? null : s"
        >
          {{ sourceKindLabel(s) }}
        </button>

        <button
          class="chip chip--fav"
          :class="{ 'chip--on': favOnly }"
          type="button"
          @click="favOnly = !favOnly"
        >
          ★ 仅看收藏
        </button>
      </div>
    </div>

    <p v-if="loadError" class="hint hint--err">{{ loadError }}</p>
    <el-skeleton v-else-if="loading && all.length === 0" :rows="6" animated />

    <template v-else>
      <!-- 选中具体领域：平铺网格 -->
      <div v-if="selectedDomain" class="grid">
        <button v-for="f in filtered" :key="f.id" class="card" type="button" @click="open(f)">
          <span class="card__name">{{ f.resultName }}</span>
          <code class="card__sym">{{ f.resultSymbol }}</code>
          <span class="card__foot">
            <SourceBadge :kind="f.source.kind" compact />
            <span class="card__domain">{{ f.domain || '未分类' }}</span>
            <span
              class="card__star"
              :class="{ 'card__star--on': favSet.has(f.id) }"
              role="button"
              :aria-label="favSet.has(f.id) ? '取消收藏' : '收藏'"
              @click="toggleFav(f.id, $event)"
              >{{ favSet.has(f.id) ? '★' : '☆' }}</span
            >
          </span>
        </button>
      </div>

      <!-- 全部：按领域分组 -->
      <template v-else>
        <section v-for="[domain, items] in grouped" :key="domain" class="group">
          <h2 class="group__title">
            {{ domain }}
            <span class="group__count">{{ items.length }}</span>
          </h2>
          <div class="grid">
            <button v-for="f in items" :key="f.id" class="card" type="button" @click="open(f)">
              <span class="card__name">{{ f.resultName }}</span>
              <code class="card__sym">{{ f.resultSymbol }}</code>
              <span class="card__foot">
                <SourceBadge :kind="f.source.kind" compact />
                <span class="card__domain">{{ f.domain || '未分类' }}</span>
                <span
                  class="card__star"
                  :class="{ 'card__star--on': favSet.has(f.id) }"
                  role="button"
                  :aria-label="favSet.has(f.id) ? '取消收藏' : '收藏'"
                  @click="toggleFav(f.id, $event)"
                  >{{ favSet.has(f.id) ? '★' : '☆' }}</span
                >
              </span>
            </button>
          </div>
        </section>
      </template>

      <div v-if="filtered.length === 0" class="empty">
        <p class="empty__title">没有匹配的公式</p>
        <p class="empty__text">
          换个关键词，或者
          <button class="link" type="button" @click="resetFilters()">清除筛选</button>
          看看全部。
        </p>
      </div>
    </template>
  </section>
</template>

<style scoped>
.view {
  display: flex;
  flex-direction: column;
  gap: var(--sp-4);
}

/* ---- 工具条 ---- */

.toolbar {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
}

.toolbar__row {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
}

/* 领域多时横向滚动，不换行 —— 换行会把内容区往下推得很远 */
.toolbar__row--chips {
  flex-wrap: nowrap;
  overflow-x: auto;
  padding-bottom: 2px;
}

.search {
  flex: 1;
  min-width: 200px;
  max-width: 420px;
  height: 34px;
  padding: 0 var(--sp-3);
  border: var(--hairline) solid var(--c-divider);
  border-radius: var(--r-btn);
  background: var(--c-surface);
  color: var(--c-text);
  font-family: inherit;
  font-size: var(--f-size-sm);
  outline: none;
}

.search:focus {
  border-color: var(--c-primary);
}

.count {
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
  font-variant-numeric: tabular-nums;
}

.toolbar__sep {
  flex-shrink: 0;
  width: var(--hairline);
  height: 18px;
  background: var(--c-divider);
}

/* 扁圆 chip */
.chip {
  flex-shrink: 0;
  height: 28px;
  padding: 0 var(--sp-3);
  border: var(--hairline) solid var(--c-divider);
  border-radius: var(--r-pill);
  background: var(--c-surface);
  color: var(--c-text-2);
  font-family: inherit;
  font-size: var(--f-size-sm);
  white-space: nowrap;
  cursor: pointer;
  transition:
    background 0.12s ease,
    border-color 0.12s ease,
    color 0.12s ease;
}

.chip:hover {
  border-color: var(--c-primary);
  color: var(--c-primary);
}

.chip--on {
  border-color: var(--c-primary);
  background: var(--c-primary);
  color: #fff;
}

.chip--on:hover {
  color: #fff;
}

.chip--fav.chip--on {
  background: var(--c-warning);
  border-color: var(--c-warning);
}

/* ---- 分组与网格 ---- */

.group + .group {
  margin-top: var(--sp-5);
}

.group__title {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  margin: 0 0 var(--sp-3);
  font-size: var(--f-size-base);
  font-weight: 600;
  color: var(--c-text);
}

.group__count {
  padding: 0 var(--sp-2);
  border-radius: var(--r-pill);
  background: var(--c-surface-2);
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
  font-weight: 400;
  line-height: 18px;
}

.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  gap: var(--sp-3);
}

/* ---- 卡片：扁平描边 + hover 抬升 ---- */

.card {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  padding: var(--sp-3) var(--sp-4);
  border: var(--hairline) solid var(--c-divider);
  border-radius: var(--r-card);
  background: var(--c-surface);
  font-family: inherit;
  text-align: left;
  cursor: pointer;
  transition:
    border-color 0.12s ease,
    background 0.12s ease,
    transform 0.12s ease;
}

.card:hover {
  border-color: var(--c-primary);
  background: var(--c-surface-hover);
  transform: translateY(-1px);
}

.card__name {
  color: var(--c-text);
  font-size: var(--f-size-base);
  font-weight: 500;
  line-height: 1.4;
  /* 最多两行，超出省略 —— 公式名长短差异很大，不限制会把网格拉得参差 */
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.card__sym {
  align-self: flex-start;
  padding: 1px var(--sp-2);
  border-radius: var(--r-sm);
  background: var(--c-surface-2);
  color: var(--c-math-symbol);
  font-family: var(--f-mono);
  font-size: var(--f-size-xs);
}

.card__foot {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  margin-top: auto;
}

.card__domain {
  flex: 1;
  min-width: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.card__star {
  flex-shrink: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-base);
  line-height: 1;
  cursor: pointer;
}

.card__star:hover,
.card__star--on {
  color: var(--c-warning);
}

/* ---- 空态与提示 ---- */

.empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--sp-2);
  padding: var(--sp-7) var(--sp-4);
  text-align: center;
}

.empty__title {
  margin: 0;
  color: var(--c-text);
  font-size: var(--f-size-lg);
  font-weight: 600;
}

.empty__text {
  margin: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
}

.hint {
  margin: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
}

.hint--err {
  color: var(--c-danger);
}

.link {
  border: none;
  background: transparent;
  padding: 0;
  color: var(--c-primary);
  font-family: inherit;
  font-size: inherit;
  cursor: pointer;
  text-decoration: underline;
}
</style>
