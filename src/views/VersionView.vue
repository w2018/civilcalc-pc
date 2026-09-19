<script setup lang="ts">
/**
 * 版本历史（P3-16）。
 *
 * 见 docs/08-IPC契约.md 组 6 与 ADR-014（head 语义）。
 *
 * ## 关键语义
 * - 版本链顺序「旧 → 新」（后端 `version_list` 已翻转好）。
 * - `head` 是「当前生效版本」；`null` 表示「最新即 head」。
 * - 「设为当前」**不改写历史**，只更新 `user_formulas.headVersion` 一列。
 * - 没有删除命令（`version.rs` 无 `version_delete`），故不提供删除。
 *
 * ## 对比
 * 先点一个版本设为 A，再点另一个设为 B，点「对比」取 6 维差异。
 * 同版本对比返回空差异（不是错误）。
 */
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { formulaList } from '@/api/formula'
import {
  versionCreate,
  versionDiff,
  versionGet,
  versionHead,
  versionList,
  versionSwitch,
} from '@/api/version'
import type { FormulaSchema, FormulaVersion, VersionDiff } from '@/types/domain'
import VersionDiffView from '@/components/version/VersionDiff.vue'
import { formatDateTime } from '@/utils/format'
import { errorMessage } from '@/types/error'

const route = useRoute()
const router = useRouter()
const formulaId = computed(() => String(route.params.id ?? ''))

const versions = ref<FormulaVersion[]>([])
const head = ref<string | null>(null)
const loading = ref(false)
const loadError = ref<string | null>(null)

const selectedA = ref<string | null>(null)
const selectedB = ref<string | null>(null)
const diff = ref<VersionDiff | null>(null)
const diffLoading = ref(false)
const diffError = ref<string | null>(null)

const showCreate = ref(false)
const changeLog = ref('')
const creating = ref(false)
const createError = ref<string | null>(null)

/** 当前生效版本号（head = null → 最新） */
const effectiveHead = computed(() =>
  head.value ?? (versions.value.length ? versions.value[versions.value.length - 1].version : null),
)

/** 每个版本的快照 schema（用于显示名称 / 表达式） */
function snapshot(v: FormulaVersion): FormulaSchema | null {
  try {
    return JSON.parse(v.schemaJson) as FormulaSchema
  } catch {
    return null
  }
}

const changeTypeLabel: Record<string, string> = {
  create: '新建',
  edit: '编辑',
  refine: '精修',
  verify: '复核',
  revert: '回退',
}

// =============================================================================
// 微调谱系（`revisedFrom`）
// =============================================================================

/**
 * ## 🔴 为什么谱系要单独算，而不是从 `version_list` 里拿
 *
 * AI 续写/微调产生的是**一条新公式**（`normalizer` 生成新的 `usr:<uuid>`，
 * 并把 `revisedFrom` 指向父公式），**不是同一条公式的版本快照** ——
 * 微调链路上从来不会调 `version_create`。
 *
 * 所以微调出来的公式在版本页里 `versions.length === 0`，页面只会说
 * 「暂无版本记录」，两条公式之间的关系完全看不见 —— 这就是
 * 「微调后的版本功能无效、没有关系链」的真正原因。
 *
 * 后端目前**没有任何命令按 `revisedFrom` 聚合**（这个字段只有写入方，
 * 没有读取方）。而 `formula_list()` 本来就返回全部 `FormulaSchema`
 * （含 `revisedFrom`），公式量级又是个人应用级别，所以直接在前端拼这棵树。
 *
 * ## 算法
 *
 * 1. 从当前公式沿 `revisedFrom` 一路向上找到**根**（最早的祖先）；
 * 2. 从根按 `revisedFrom` 做一次 DFS，得到整棵谱系（含分叉）；
 * 3. 输出「根 → 叶」顺序，带 `depth` 用于缩进。
 *
 * ⚠️ 用 `visited` 兜住成环：`revisedFrom` 正常是树，但脏数据可能指回自己。
 */
interface LineageNode {
  id: string
  name: string
  createdAt: number
  /** 距根的层级（根 = 0），用于缩进 */
  depth: number
  /** 是不是当前正在看的这条 */
  current: boolean
}

const lineage = ref<LineageNode[]>([])

/** 纯函数：从全部公式里解出 `currentId` 所在的那棵谱系 */
function buildLineage(all: FormulaSchema[], currentId: string): LineageNode[] {
  const byId = new Map(all.map((s) => [s.id, s]))
  const cur = byId.get(currentId)
  if (!cur) return []

  // ① 向上找根
  const seen = new Set<string>()
  let root = cur
  while (root.revisedFrom && !seen.has(root.id)) {
    seen.add(root.id)
    const parent = byId.get(root.revisedFrom)
    if (!parent) break
    root = parent
  }

  // ② 子表（按创建时间排序，保证同一层里顺序稳定）
  const children = new Map<string, FormulaSchema[]>()
  for (const s of all) {
    if (!s.revisedFrom) continue
    const list = children.get(s.revisedFrom) ?? []
    list.push(s)
    children.set(s.revisedFrom, list)
  }
  for (const list of children.values()) list.sort((a, b) => a.createdAt - b.createdAt)

  // ③ DFS
  const out: LineageNode[] = []
  const visited = new Set<string>()
  const walk = (node: FormulaSchema, depth: number): void => {
    if (visited.has(node.id)) return
    visited.add(node.id)
    out.push({
      id: node.id,
      name: node.resultName || '未命名公式',
      createdAt: node.createdAt,
      depth,
      current: node.id === currentId,
    })
    for (const c of children.get(node.id) ?? []) walk(c, depth + 1)
  }
  walk(root, 0)
  return out
}

/**
 * 取谱系。
 *
 * 单独失败、单独静默 —— 它只是辅助信息，读不到不该影响版本页的主功能。
 */
async function loadLineage(): Promise<void> {
  if (!formulaId.value) return
  try {
    const all = await formulaList()
    lineage.value = buildLineage(all, formulaId.value)
  } catch (e) {
    console.warn('[VersionView] 读微调谱系失败', e)
    lineage.value = []
  }
}

/** 跳到谱系里的另一条公式 */
function openFormula(id: string): void {
  router.push({ name: 'formula', params: { id } })
}

async function load() {
  if (!formulaId.value) return
  loading.value = true
  loadError.value = null
  try {
    const [list, h] = await Promise.all([
      versionList(formulaId.value),
      versionHead(formulaId.value),
    ])
    versions.value = list
    head.value = h
    // 默认选最新两版做对比
    if (list.length >= 2) {
      selectedA.value = list[list.length - 2].version
      selectedB.value = list[list.length - 1].version
    } else if (list.length === 1) {
      selectedA.value = list[0].version
      selectedB.value = list[0].version
    }
  } catch (e) {
    loadError.value = errorMessage(e as never)
  } finally {
    loading.value = false
  }

  // 谱系是另一套数据（`revisedFrom`，不是版本快照）—— 单独取、单独失败
  void loadLineage()
}

async function runDiff() {
  if (!formulaId.value || !selectedA.value || !selectedB.value) return
  diffLoading.value = true
  diffError.value = null
  try {
    diff.value = await versionDiff(formulaId.value, selectedA.value, selectedB.value)
  } catch (e) {
    diffError.value = errorMessage(e as never)
  } finally {
    diffLoading.value = false
  }
}

async function setHead(v: string) {
  if (!formulaId.value) return
  try {
    await versionSwitch(formulaId.value, v)
    head.value = v
  } catch (e) {
    loadError.value = errorMessage(e as never)
  }
}

async function createVersion() {
  if (!formulaId.value) return
  creating.value = true
  createError.value = null
  try {
    // 取当前生效版本的快照作为新版本的基础 schema
    const base =
      effectiveHead.value != null
        ? await versionGet(formulaId.value, effectiveHead.value)
        : null
    const schema = base ? snapshot(base) : null
    if (!schema) {
      createError.value = '无法读取当前版本快照'
      return
    }
    await versionCreate(schema, changeLog.value.trim(), 'user')
    changeLog.value = ''
    showCreate.value = false
    await load()
  } catch (e) {
    createError.value = errorMessage(e as never)
  } finally {
    creating.value = false
  }
}

function back() {
  router.push({ name: 'formula', params: { id: formulaId.value } })
}

/**
 * 时间显示统一走 `utils/format`。
 *
 * ⚠️ 这里曾经自己写了一个 `fmtTime(ts)`，把入参当**秒**、`new Date(ts * 1000)` ——
 * 而后端给的是**毫秒**，于是界面上显示成 `58684/11/23`。
 * 不要再在这里自己格式化时间。
 */

/**
 * 已经加载过的公式 id。
 *
 * `useRoute()` 是**全局响应式**的：切走时 `route.params.id` 变空串、
 * 切回来又变回原值，于是 `watch(formulaId, load)` 会**每次切回来都重载**，
 * 把「选了哪两版做对比」重置掉。记一下就能挡掉多余的第二次。
 *
 * （`load()` 自己会判空 id 直接返回，所以切走那次不会误清数据。）
 */
let loadedId = ''

watch(formulaId, (id) => {
  if (!id || id === loadedId) return
  loadedId = id
  void load()
})

onMounted(() => {
  if (!formulaId.value) return
  loadedId = formulaId.value
  void load()
})
</script>

<template>
  <section class="view">
    <header class="view__bar">
      <button class="link" type="button" @click="back">← 返回公式</button>
      <h1 class="view__title">版本历史</h1>
      <button class="btn btn--primary" type="button" @click="showCreate = true">
        保存新版本
      </button>
    </header>

    <p v-if="loadError" class="view__err">{{ loadError }}</p>
    <p v-else-if="loading" class="view__hint">加载中…</p>

    <!-- 微调谱系：独立于版本快照渲染 ——
         微调出来的公式本身没有版本快照，挂在 `v-else` 里就永远看不见 -->
    <div v-if="lineage.length > 1" class="lin">
      <div class="lin__head">
        <span class="lin__title">微调谱系</span>
        <span class="lin__hint">
          AI 续写 / 微调产生的是新公式，靠这条链串起来（不是版本快照）
        </span>
      </div>
      <ol class="lin__list">
        <li
          v-for="n in lineage"
          :key="n.id"
          class="lin__item"
          :style="{ paddingLeft: `calc(${n.depth} * var(--sp-5))` }"
        >
          <button
            class="lin__node"
            type="button"
            :class="{ 'lin__node--current': n.current }"
            :disabled="n.current"
            :title="n.current ? '当前正在看的公式' : '打开这条公式'"
            @click="openFormula(n.id)"
          >
            <span class="lin__name">{{ n.name }}</span>
            <span v-if="n.current" class="lin__badge">当前</span>
            <span class="lin__time">{{ formatDateTime(n.createdAt) }}</span>
          </button>
        </li>
      </ol>
    </div>

    <p v-if="versions.length === 0" class="view__hint">暂无版本快照</p>

    <div v-else class="vv">
      <!-- 版本链 -->
      <ol class="vv__list">
        <li
          v-for="v in versions"
          :key="v.version"
          class="vv__item"
          :class="{ 'vv__item--head': v.version === effectiveHead }"
        >
          <div class="vv__meta">
            <span class="vv__ver">v{{ v.version }}</span>
            <span v-if="v.version === effectiveHead" class="vv__badge">当前</span>
            <span class="vv__type">{{ changeTypeLabel[v.changeType] ?? v.changeType }}</span>
          </div>

          <div class="vv__name">{{ snapshot(v)?.resultName ?? '（未命名）' }}</div>
          <code class="vv__expr">{{ snapshot(v)?.expression ?? '—' }}</code>

          <div class="vv__sub">
            <span>{{ v.editor || '—' }}</span>
            <span>{{ formatDateTime(v.createdAt) }}</span>
          </div>
          <div v-if="v.changeLog" class="vv__log">「{{ v.changeLog }}」</div>

          <div class="vv__acts">
            <button
              class="link"
              type="button"
              :disabled="v.version === effectiveHead"
              @click="setHead(v.version)"
            >
              设为当前
            </button>
            <label class="vv__pick">
              <input
                type="radio"
                :name="'verA'"
                :value="v.version"
                :checked="selectedA === v.version"
                @change="selectedA = v.version"
              />
              A
            </label>
            <label class="vv__pick">
              <input
                type="radio"
                :name="'verB'"
                :value="v.version"
                :checked="selectedB === v.version"
                @change="selectedB = v.version"
              />
              B
            </label>
          </div>
        </li>
      </ol>

      <!-- 对比 -->
      <div class="vv__diff">
        <div class="vv__diff-head">
          <span>对比 A=v{{ selectedA }} 与 B=v{{ selectedB }}</span>
          <button
            class="btn"
            type="button"
            :disabled="!selectedA || !selectedB || diffLoading"
            @click="runDiff"
          >
            {{ diffLoading ? '对比中…' : '对比' }}
          </button>
        </div>
        <p v-if="diffError" class="view__err">{{ diffError }}</p>
        <VersionDiffView :diff="diff" :label-a="`v${selectedA}`" :label-b="`v${selectedB}`" />
      </div>
    </div>

    <!-- 保存新版本 -->
    <div v-if="showCreate" class="modal" @click.self="showCreate = false">
      <div class="modal__card">
        <h2 class="modal__title">保存新版本</h2>
        <p class="modal__hint">基于当前生效版本（v{{ effectiveHead }}）创建快照。</p>
        <textarea
          v-model="changeLog"
          class="modal__input"
          rows="3"
          placeholder="变更说明（可选）：如「修正单位换算」"
        />
        <p v-if="createError" class="view__err">{{ createError }}</p>
        <div class="modal__acts">
          <button class="btn" type="button" @click="showCreate = false">取消</button>
          <button
            class="btn btn--primary"
            type="button"
            :disabled="creating"
            @click="createVersion"
          >
            {{ creating ? '保存中…' : '保存' }}
          </button>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
.view {
  display: flex;
  flex-direction: column;
  gap: var(--sp-4);
  padding: var(--sp-5) var(--sp-4);
}
.view__bar {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
}
.view__title {
  margin: 0;
  flex: 1;
  font-size: var(--f-size-xl);
  font-weight: 600;
  color: var(--c-text);
}
.view__hint,
.view__err {
  margin: 0;
  font-size: var(--f-size-sm);
}
.view__err {
  color: var(--c-danger, #e54545);
}

/* ---- 微调谱系 ---- */
.lin {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  padding: var(--sp-3) var(--sp-4);
  border-radius: var(--r-card);
  background: var(--c-surface-2);
}
.lin__head {
  display: flex;
  align-items: baseline;
  gap: var(--sp-3);
  flex-wrap: wrap;
}
.lin__title {
  font-size: var(--f-size-sm);
  font-weight: 600;
  color: var(--c-text);
}
.lin__hint {
  font-size: var(--f-size-xs);
  color: var(--c-text-3);
}
.lin__list {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
}
/* 缩进由内联 style 按 depth 给（见模板） */
.lin__node {
  display: inline-flex;
  align-items: center;
  gap: var(--sp-2);
  max-width: 100%;
  padding: var(--sp-1) var(--sp-3);
  border: var(--hairline) solid var(--c-divider);
  border-radius: var(--r-pill);
  background: var(--c-surface);
  color: var(--c-text);
  font-size: var(--f-size-sm);
  text-align: left;
  cursor: pointer;
}
.lin__node:hover:not(:disabled) {
  border-color: var(--c-primary);
  color: var(--c-primary);
}
.lin__node--current {
  border-color: var(--c-primary);
  background: var(--c-primary-light);
  color: var(--c-primary);
  cursor: default;
  /* 浏览器会给 disabled 按钮压一层灰，这里显式压回来 */
  opacity: 1;
}
.lin__name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.lin__badge {
  flex-shrink: 0;
  padding: 0 var(--sp-1);
  border-radius: var(--r-sm);
  background: var(--c-primary);
  color: #fff;
  font-size: var(--f-size-xs);
}
.lin__time {
  flex-shrink: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.vv {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  gap: var(--sp-5);
  align-items: start;
}
@media (max-width: 860px) {
  .vv {
    grid-template-columns: 1fr;
  }
}
.vv__list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
}
.vv__item {
  border: var(--hairline) solid var(--c-divider);
  border-radius: var(--r-card);
  background: var(--c-surface);
  padding: var(--sp-3);
}
.vv__item--head {
  border-color: var(--c-accent);
}
.vv__meta {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
}
.vv__ver {
  font-family: var(--f-mono);
  font-weight: 600;
  color: var(--c-text);
}
.vv__badge {
  font-size: var(--f-size-xs);
  color: var(--c-text-inverse);
  background: var(--c-accent);
  border-radius: var(--r-pill);
  padding: 0 var(--sp-2);
}
.vv__type {
  font-size: var(--f-size-xs);
  color: var(--c-text-3);
}
.vv__name {
  margin-top: var(--sp-1);
  font-weight: 500;
  color: var(--c-text);
}
.vv__expr {
  display: block;
  margin-top: var(--sp-1);
  font-family: var(--f-mono);
  font-size: var(--f-size-xs);
  color: var(--c-text-2);
  white-space: pre-wrap;
  word-break: break-all;
}
.vv__sub {
  margin-top: var(--sp-1);
  display: flex;
  gap: var(--sp-3);
  font-size: var(--f-size-xs);
  color: var(--c-text-3);
}
.vv__log {
  margin-top: var(--sp-1);
  font-size: var(--f-size-sm);
  color: var(--c-text-2);
  font-style: italic;
}
.vv__acts {
  margin-top: var(--sp-2);
  display: flex;
  align-items: center;
  gap: var(--sp-3);
}
.vv__pick {
  display: inline-flex;
  align-items: center;
  gap: var(--sp-1);
  font-size: var(--f-size-sm);
  color: var(--c-text-2);
}
.vv__diff {
  border: var(--hairline) solid var(--c-divider);
  border-radius: var(--r-card);
  background: var(--c-surface);
  padding: var(--sp-3);
  position: sticky;
  top: var(--sp-3);
}
.vv__diff-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--sp-3);
  margin-bottom: var(--sp-3);
  font-size: var(--f-size-sm);
  color: var(--c-text-2);
}

/* 模态 */
.modal {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.4);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 50;
}
.modal__card {
  width: min(420px, 92vw);
  background: var(--c-surface);
  border-radius: var(--r-card);
  padding: var(--sp-5);
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
}
.modal__title {
  margin: 0;
  font-size: var(--f-size-lg);
  color: var(--c-text);
}
.modal__hint {
  margin: 0;
  font-size: var(--f-size-sm);
  color: var(--c-text-3);
}
.modal__input {
  font-family: inherit;
  font-size: var(--f-size-sm);
  border: var(--hairline) solid var(--c-divider);
  border-radius: var(--r-control);
  padding: var(--sp-2);
  color: var(--c-text);
  background: var(--c-surface-2, var(--c-surface));
  resize: vertical;
}
.modal__acts {
  display: flex;
  justify-content: flex-end;
  gap: var(--sp-3);
}
.btn {
  font: inherit;
  font-size: var(--f-size-sm);
  border: var(--hairline) solid var(--c-divider);
  border-radius: var(--r-control);
  padding: var(--sp-2) var(--sp-4);
  background: var(--c-surface);
  color: var(--c-text);
  cursor: pointer;
}
.btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.btn--primary {
  background: var(--c-accent);
  border-color: var(--c-accent);
  color: var(--c-text-inverse);
}
.link {
  font: inherit;
  font-size: var(--f-size-sm);
  border: none;
  background: none;
  color: var(--c-accent);
  cursor: pointer;
  padding: 0;
}
.link:disabled {
  color: var(--c-text-3);
  cursor: not-allowed;
}
</style>
