<script setup lang="ts">
/**
 * 公式工作台 —— **本应用的核心页面**。
 *
 * 见 docs/05-项目开发方案.md §2.1.2 / §3.3.3。
 *
 * ## 页面结构
 *
 * ```
 * [公式头部]  名称 / 来源徽章 / 领域 / 收藏 / 版本 / 表达式
 * [思考过程]  ThinkingBox（AI 解析时的思考，来自最近一次历史）
 * [参数区]    ParamForm（可折叠；**只由用户手动收起**）
 * [结果区]    ResultPanel（主结果 / 告警 / 多结果 / 分支）
 * [计算过程]  StepsList（分步 + Excel 公式）
 * [详解]      ExplanationPanel（分步依据 + {{img:N}} 内联图）
 * [Excel]     ExcelPanel（两模式切换）
 * ```
 *
 * ## 🔴 参数区**不做自动收起**
 *
 * 曾经的做法是「首次求值成功后自动收起参数区」（源项目行为）。
 * 实际上很碍事：用户输完一个参数、算出来 → 参数区自己收起来，
 * 想再改一个就得先点开；连续试算几个取值时每次都要多点一下。
 * 现在收起**只由用户手动触发**（点标题行）。
 *
 * ## 历史写入是显式动作
 *
 * 实时求值走 `eval_schema`（不写历史）；用户点「记录本次计算」
 * 才走 `eval_formula` 写一条。见 `stores/formula.ts` 的说明。
 *
 * ## 思考内容从哪来
 *
 * 思考内容**不在公式上**，而在历史记录里（`thinkingContent` 字段）。
 * 所以这里用 `history_thinking(formulaId)` 取「最近一次」的思考 ——
 * 与源项目 `getLatestThinkingContent` 同语义。
 * 最新那条没有思考内容时返回 `null`（后端**不过滤空值**），此时不显示思考框。
 */
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { favoriteApi, historyApi } from '@/api'
import FormulaHeader from '@/components/formula/FormulaHeader.vue'
import ParamForm from '@/components/formula/ParamForm.vue'
import ResultPanel from '@/components/result/ResultPanel.vue'
import StepsList from '@/components/steps/StepsList.vue'
import ExcelPanel from '@/components/excel/ExcelPanel.vue'
import ExportDialog from '@/components/report/ExportDialog.vue'
import ThinkingBox from '@/components/llm/ThinkingBox.vue'
import ExplanationPanel from '@/components/llm/ExplanationPanel.vue'
import RefineDialog from '@/components/llm/RefineDialog.vue'
import { useFormulaStore } from '@/stores/formula'
import { useThinkingStore } from '@/stores/thinking'
import { useHotkeys } from '@/composables/useHotkeys'
import { errorMessage } from '@/types/error'
import type { FormulaExplanation } from '@/types/domain'

const route = useRoute()
const router = useRouter()
const store = useFormulaStore()
/** 刚生成出来的思考暂存（需求 4：历史里还没有时靠它） */
const thinkingStore = useThinkingStore()

const formulaId = computed(() => String(route.params.id ?? ''))

/**
 * 历史回填 id（`/formula/:id?historyId=123`）。
 *
 * 路由里**只带 id 不带 JSON** —— 一次计算的 `inputsJson` 可能几十 KB，
 * 塞进 URL 会让历史记录膨胀且难读。工作台拿到 id 后自己查。
 */
const historyId = computed<number | null>(() => {
  const raw = route.query.historyId
  const s = Array.isArray(raw) ? raw[0] : raw
  if (!s) return null
  const n = Number(s)
  return Number.isFinite(n) ? n : null
})

const favorite = ref(false)
/**
 * 参数区是否收起。
 *
 * ⚠️ **只由用户手动切换**，求值成功不会自动收起（见文件头说明）。
 * 换一条公式时回到展开态（`load()` 里重置）。
 */
const paramsCollapsed = ref(false)
/** Excel 公式区是否收起（需求 5：默认收起 —— 它是「拿走用」的辅助信息，不是主线） */
const excelCollapsed = ref(true)

/** 参数表单实例（用于 Domain 错误时聚焦字段） */
const paramFormRef = ref<{ focusField: (symbol: string) => void } | null>(null)

// ---------------------------------------------------------------- 加载

/** AI 解析时的思考内容（来自最近一次历史；没有则为空串） */
const thinking = ref('')

/** 详解（可被 ExplanationPanel 重新生成后替换） */
const explanation = ref<FormulaExplanation | null>(null)

async function load(id: string, fromHistory: number | null): Promise<void> {
  if (!id) return
  paramsCollapsed.value = false
  excelCollapsed.value = true
  thinking.value = ''
  window.addEventListener('beforeunload', onBeforeUnload)
  await store.load(id, fromHistory)

  if (store.loadError) {
    // 公式不存在（被删了 / 重置清掉了）：把「上次用的公式」标记清掉，
    // 否则侧栏的「公式计算」会一直指向这条不存在的公式，
    // 每次点都落到「公式加载失败」页。
    if (store.loadError.kind === 'notFound') store.forgetWorkspace()
    return
  }

  explanation.value = store.schema?.explanation ?? null

  try {
    favorite.value = await favoriteApi.isFavorite(id)
  } catch (e) {
    console.warn('[FormulaView] 读收藏状态失败', e)
  }

  // 思考内容的两个来源（需求 4）：
  //   ① 历史里最近一次计算的 `thinkingContent`（持久，优先）
  //   ② 刚生成完还没算过时，`thinkingStore` 里的会话暂存
  // 后端「不过滤空值」：最新那条没有思考就返回 null，此时才回落 ②。
  try {
    thinking.value = (await historyApi.historyThinking(id)) ?? ''
  } catch (e) {
    console.warn('[FormulaView] 读思考内容失败', e)
  }
  if (!thinking.value) thinking.value = thinkingStore.recall(id)
}

/**
 * 已经加载过的 `(公式 id, 历史 id)`。
 *
 * ## 🔴 为什么必须有这个「已加载」标记
 *
 * `useRoute()` 返回的是**全局响应式**的当前路由，不是「组件挂载时的那条路由」。
 * 所以本页被 `keep-alive` 缓存、用户切到别的标签时：
 *
 * ```
 * /formula/a  →  切到 /library        →  route.params.id 变成 ''   → 触发 watch
 *             →  切回 /formula/a      →  route.params.id 变回 'a'  → 又触发 watch
 * ```
 *
 * 于是**每次切回来都会重新 `load()` 一次** —— 参数被重建成草稿/默认值、
 * 结果重新求值、思考重新读、折叠状态被重置。表现就是
 * 「切出去再切回来，状态丢了」，而组件其实一直活着。
 *
 * 记下「这份 key 已经加载过」就能把第二次触发挡掉。
 */
let loadedKey = ''

// `historyId` 也要 watch —— 用户可能在**同一个公式**下点另一条历史
watch(
  [formulaId, historyId],
  ([id, hid]) => {
    // 切走时 `id` 会变成空串：什么都不做，更不要清状态
    if (!id) return
    const key = `${id}#${hid ?? ''}`
    // 同一份内容已经加载过 → 直接返回（这就是「切回来不重载」）
    if (key === loadedKey) return
    loadedKey = key
    void load(id, hid)
  },
  { immediate: true },
)

onBeforeUnmount(() => {
  window.removeEventListener('beforeunload', onBeforeUnload)
  // 落草稿 + 清状态（离开工作台）
  void store.reset()
})

// ---------------------------------------------------------------- 交互

// 参数区**不做自动收起** —— 只由用户点标题行切换。
// 详见文件头「参数区不做自动收起」。

// ---------------------------------------------------------------- 历史自动记录

/**
 * 自动记录本次计算（需求 5）。
 *
 * ## 为什么是「稳定 1.2s 后记」而不是「每次求值都记」
 *
 * 用户拖参数时求值是**连续**触发的（每次改动都会重算）。
 * 如果每次成功求值都写一条历史，拖一次滑块就能灌几十条，
 * 历史页会立刻变得没法用。
 *
 * 所以策略是：**参数稳定下来（1.2s 没再变）且求值成功**才写一条。
 * 这与「用户停手了，这一次的结果是他要的」的直觉一致。
 *
 * ## 去重
 *
 * 用「符号=原文」拼成签名，**同一组参数只记一次** ——
 * 否则反复切走再切回来（重新挂载 → 重新求值）会重复写。
 *
 * ## 手动按钮仍然保留
 *
 * 想立刻记一条（不等 1.2s）时用它。两条路径都走
 * `store.confirmCompute()` —— 写历史**只有这一个出口**。
 */
let autoRecordTimer: number | null = null
/** 上一次已记录过的参数签名（避免重复写） */
let lastRecordedSig = ''

/** 当前参数的签名（顺序即变量顺序，稳定可比） */
function inputSignature(): string {
  return store.variables
    .map((v) => `${v.symbol}=${(store.paramValues[v.symbol] ?? '').trim()}`)
    .join('&')
}

async function autoRecord(): Promise<void> {
  if (!store.canCompute || store.evalStatus !== 'success') return
  const sig = inputSignature()
  if (sig === lastRecordedSig) return

  // 带上思考内容（需求 4）——这样它随历史一起落库，
  // 之后即使会话暂存没了（重启应用），公式页仍能从历史里读到。
  const r = await store.confirmCompute(thinking.value || undefined)
  // 只有真的写成功才记住签名，否则下次还有机会补上
  if (r) lastRecordedSig = sig
}

/** 参数或求值状态变化 → 重置防抖计时器 */
watch(
  () => [store.evalStatus, inputSignature()] as const,
  () => {
    if (autoRecordTimer !== null) window.clearTimeout(autoRecordTimer)
    autoRecordTimer = window.setTimeout(() => {
      autoRecordTimer = null
      void autoRecord()
    }, 1200)
  },
)

onBeforeUnmount(() => {
  if (autoRecordTimer !== null) {
    window.clearTimeout(autoRecordTimer)
    autoRecordTimer = null
  }
})

/** 后端返回 `domain` 错误 → 聚焦对应字段（文档硬约束） */
watch(
  () => store.focusedErrorKey,
  (key) => {
    if (key) {
      // 字段被收起时先展开，否则聚焦不可见
      paramsCollapsed.value = false
      paramFormRef.value?.focusField(key)
    }
  },
)

async function toggleFavorite(): Promise<void> {
  if (!formulaId.value) return
  try {
    favorite.value = await favoriteApi.toggleFavorite(formulaId.value)
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  }
}

/**
 * 参数框失焦：求值 + **立即落盘草稿**（需求 2）。
 *
 * 草稿平时是 500ms 防抖落盘的；失焦是一个明确的「这个字段我填完了」信号，
 * 顺手把草稿写掉能把「刚打完字就关窗」的丢失窗口从 500ms 缩到几乎为零。
 */
function onParamBlur(symbol: string): void {
  void store.evaluateOnBlur(symbol)
  void store.flushDraft()
}

/**
 * 关窗前把待落盘的草稿写掉（需求 2：实时持久化）。
 *
 * ## 为什么需要它
 *
 * 关窗**不会卸载 Vue 组件** —— `onBeforeUnmount` 里的 `flushDraft()` 不会跑，
 * 进程直接就没了。所以防抖窗口内（500ms）的输入会丢。
 *
 * ⚠️ 这是**尽力而为**：`beforeunload` 里没法 await 异步 IPC，
 * 只能发出去。真正兜底的是上面「失焦即落盘」——
 * 那条覆盖了绝大多数实际操作路径。
 */
function onBeforeUnload(): void {
  void store.flushDraft()
}

/** 记录本次计算（写历史）—— 唯一写历史的入口 */
const recording = ref(false)
async function record(): Promise<void> {
  if (recording.value) return
  recording.value = true
  try {
    const r = await store.confirmCompute(thinking.value || undefined)
    if (r) ElMessage.success('已记录本次计算')
    else ElMessage.warning('当前参数有误，未记录')
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  } finally {
    recording.value = false
  }
}

/** 复制成功提示 */
function onCopied(): void {
  ElMessage.success('已复制')
}

const hasSteps = computed(() => (store.result?.stepResults.length ?? 0) > 0)

/** 导出对话框开关 */
const exportOpen = ref(false)

/** AI 微调对话框开关（需求 6） */
const refineOpen = ref(false)

/** 微调结果保存成功 → 直接打开那条新公式 */
function onRefined(id: string): void {
  // 用户刚在弹窗里点了「保存为新公式」，意图明确 ——
  // 这里再弹一次「要切换公式吗」是多余的。先清掉占用标记再跳。
  store.forgetWorkspace()
  router.push({ name: 'formula', params: { id } })
}

/** 详解生成完成后写回 schema 并落库 */
const savingExplanation = ref(false)
async function onExplanationGenerated(e: FormulaExplanation): Promise<void> {
  const s = store.schema
  if (!s || savingExplanation.value) return

  savingExplanation.value = true
  explanation.value = e
  try {
    store.replaceSchema({ ...s, explanation: e, updatedAt: Date.now() })
    await store.save()
    ElMessage.success('详解已保存')
  } catch (err) {
    ElMessage.error(errorMessage(err as never))
  } finally {
    savingExplanation.value = false
  }
}

// ---------------------------------------------------------------- 快捷键

/**
 * `Ctrl+E` 导出 / `Ctrl+S` 保存 / `Ctrl+Enter` 求值。
 *
 * `enabled` 挂在「公式已加载」上：`Ctrl+S` 在没有公式时执行
 * 会把 `null` 写进数据库（`store.save()` 里有保护，但不如这里直接不响应）。
 */
useHotkeys(
  {
    'ctrl+e': () => {
      exportOpen.value = true
    },
    'ctrl+s': () => {
      void saveFormula()
    },
    'ctrl+enter': () => {
      store.triggerEvaluate()
    },
  },
  () => store.schema !== null,
)

/** `Ctrl+S`：保存公式（含把参数草稿落盘） */
async function saveFormula(): Promise<void> {
  if (!store.schema) return
  try {
    await store.save()
    ElMessage.success('已保存')
  } catch (err) {
    ElMessage.error(errorMessage(err as never))
  }
}
</script>

<template>
  <div class="view">
    <!-- 未选公式（从左侧「公式计算」直接进来）——
         给三个入口让用户挑一条，而不是显示空白 -->
    <section v-if="!formulaId" class="panel empty-home">
      <h2 class="empty-home__title">公式计算</h2>
      <p class="empty-home__text">
        这里用来计算已经有的公式。从下面任一处选一条，或先让 AI 生成一条新的。
      </p>
      <div class="empty-home__ops">
        <el-button @click="router.push({ name: 'favorites' })">⭐ 收藏</el-button>
        <el-button @click="router.push({ name: 'history' })">🕘 历史</el-button>
        <el-button @click="router.push({ name: 'library' })">📚 公式库</el-button>
        <el-button type="primary" @click="router.push({ name: 'query' })">✨ 新建公式</el-button>
      </div>
    </section>

    <!-- 加载失败 / 公式不存在 -->
    <el-result
      v-else-if="store.loadError"
      icon="warning"
      title="公式加载失败"
      :sub-title="errorMessage(store.loadError)"
    >
      <template #extra>
        <!-- 搜索页已并入「新建公式」（需求 10），这里回公式库 -->
        <el-button @click="router.push({ name: 'library' })">返回公式库</el-button>
      </template>
    </el-result>

    <template v-else-if="store.schema">
      <FormulaHeader
        :schema="store.schema"
        :favorite="favorite"
        @toggle-favorite="toggleFavorite()"
        @open-versions="router.push({ name: 'versions', params: { id: formulaId } })"
      />

      <div class="view__actions">
        <span class="view__keys">
          <kbd>Ctrl</kbd>+<kbd>Enter</kbd> 求值 ·
          <kbd>Ctrl</kbd>+<kbd>S</kbd> 保存 ·
          <kbd>Ctrl</kbd>+<kbd>E</kbd> 导出
        </span>
        <button class="link" type="button" @click="refineOpen = true">AI 微调</button>
        <button class="link" type="button" @click="exportOpen = true">导出计算书</button>
      </div>

      <!-- AI 微调：基于当前公式改写，结果存为新公式（需求 6） -->
      <RefineDialog
        v-model="refineOpen"
        :schema="store.schema"
        @saved="onRefined"
      />

      <ExportDialog
        v-model="exportOpen"
        :formula-id="formulaId"
        :schema="store.schema"
        :param-values="store.paramValues"
      />

      <!-- AI 思考过程（最近一次历史里有才显示） -->
      <ThinkingBox :text="thinking" title="AI 解析时的思考" sticky />

      <!-- 参数区 -->
      <section class="panel">
        <button
          class="panel__head"
          type="button"
          :aria-expanded="!paramsCollapsed"
          @click="paramsCollapsed = !paramsCollapsed"
        >
          <span class="panel__caret" aria-hidden="true">{{ paramsCollapsed ? '▸' : '▾' }}</span>
          <span class="panel__title">参数</span>
          <span class="panel__count">{{ store.variables.length }}</span>
          <span v-if="store.hasMissingRequired" class="panel__badge">有必填项未填</span>
        </button>

        <div v-show="!paramsCollapsed" class="panel__body">
          <ParamForm
            ref="paramFormRef"
            :variables="store.variables"
            :values="store.paramValues"
            :evaluated="store.paramEvaluated"
            :errors="store.paramErrors"
            :focus-key="store.focusedErrorKey"
            @change="store.setParam"
            @blur="onParamBlur"
            @operator="store.appendOperator"
          />
        </div>
      </section>

      <!-- 结果区 -->
      <section class="panel">
        <div class="panel__head panel__head--static">
          <span class="panel__title">结果</span>
          <button
            v-if="store.evalStatus === 'success'"
            class="link"
            type="button"
            :disabled="recording"
            @click="record()"
          >
            记录本次计算
          </button>
        </div>
        <div class="panel__body">
          <ResultPanel
            :schema="store.schema"
            :result="store.result"
            :status="store.evalStatus"
            :eval-error="store.evalError"
            :command-error="store.commandError"
            @copied="onCopied"
          />
        </div>
      </section>

      <!-- 计算过程 -->
      <section v-if="hasSteps" class="panel">
        <div class="panel__head panel__head--static">
          <span class="panel__title">计算过程</span>
        </div>
        <div class="panel__body">
          <StepsList :steps="store.result?.stepResults ?? []" @copied="onCopied" />
        </div>
      </section>

      <!-- 计算公式详解（AI 生成；没有时可在此生成） -->
      <ExplanationPanel
        :schema="store.schema"
        :explanation="explanation"
        @generated="onExplanationGenerated"
      />

      <!-- Excel 公式（需求 5：默认折叠，可展开） -->
      <section class="panel">
        <button
          class="panel__head"
          type="button"
          :aria-expanded="!excelCollapsed"
          @click="excelCollapsed = !excelCollapsed"
        >
          <span class="panel__caret" aria-hidden="true">{{ excelCollapsed ? '▸' : '▾' }}</span>
          <span class="panel__title">Excel 公式</span>
          <span class="panel__hint">复制到 Excel 里直接算</span>
        </button>

        <div v-show="!excelCollapsed" class="panel__body">
          <ExcelPanel
            :schema="store.schema"
            :param-values="store.paramValues"
            @copied="onCopied"
          />
        </div>
      </section>
    </template>

    <!-- 首次加载骨架 -->
    <el-skeleton v-else-if="store.loading" :rows="6" animated />
  </div>
</template>

<style scoped>
.view {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  max-width: none;
}

/* 「公式计算」未选公式时的空态 */
.empty-home {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--sp-3);
  padding: var(--sp-7) var(--sp-5);
  text-align: center;
}

.empty-home__title {
  margin: 0;
  font-size: var(--f-size-xl);
  font-weight: 600;
  color: var(--c-text);
}

.empty-home__text {
  margin: 0;
  max-width: 460px;
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
  line-height: 1.8;
}

.empty-home__ops {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: var(--sp-2);
  margin-top: var(--sp-2);
}

.panel {
  border-radius: var(--r-card);
  background: var(--c-surface);
}

.view__actions {  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: var(--sp-4);
}

.view__keys {
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.view__keys kbd {
  padding: 0 4px;
  border: var(--hairline) solid var(--c-divider);
  border-radius: var(--r-sm);
  background: var(--c-surface-2);
  font-family: var(--f-mono);
  font-size: 11px;
}

.panel__head {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  width: 100%;
  padding: var(--sp-4);
  border: none;
  background: transparent;
  font-family: inherit;
  font-size: var(--f-size-base);
  color: var(--c-text);
  cursor: pointer;
  text-align: left;
}

/* 静态标题行（结果区 / 步骤区不需要折叠） */
.panel__head--static {
  cursor: default;
  justify-content: space-between;
  padding-bottom: 0;
}

.panel__caret {
  width: 12px;
  flex-shrink: 0;
  color: var(--c-text-3);
}

.panel__title {
  font-weight: 600;
}

.panel__count {
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
}

.panel__badge {
  margin-left: auto;
  padding: 1px var(--sp-2);
  border-radius: var(--r-pill);
  background: var(--c-surface-2);
  color: var(--c-warning);
  font-size: var(--f-size-xs);
}

/* 折叠标题行的补充说明（如 Excel 公式区的用途提示） */
.panel__hint {
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.panel__body {
  padding: 0 var(--sp-4) var(--sp-4);
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

.link:disabled {
  color: var(--c-text-3);
  cursor: default;
  text-decoration: none;
}
</style>
