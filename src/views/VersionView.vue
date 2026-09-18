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

function fmtTime(ts: number): string {
  const d = new Date(ts * 1000)
  if (Number.isNaN(d.getTime())) return '—'
  return d.toLocaleString()
}

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
    <p v-else-if="versions.length === 0" class="view__hint">暂无版本记录</p>

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
            <span>{{ fmtTime(v.createdAt) }}</span>
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
