<script setup lang="ts">
/**
 * 新建公式 —— 单一入口：**粘公式 / 粘 JSON / 写需求**都在这一个框里。
 *
 * 见 docs/05-项目开发方案.md §2.1.2 与「流程 1」。
 *
 * ## 🔴 为什么合并了「粘贴」与「描述」（需求 11）
 *
 * 原来两个页面：
 *
 * | 页面 | 输入 | 走不走 AI |
 * |---|---|---|
 * | `/paste` | 公式表达式或 Schema JSON | 不走（本地解析） |
 * | `/query` | 自然语言描述（+ 附图） | 走 `normalize_from_query` |
 *
 * 问题是**用户得先判断自己手里的是哪一种**，然后才选页面 ——
 * 而这个判断本该由程序做。所以合并成一页，提交时自动分流：
 *
 * 1. 内容像 **Schema JSON** → 本地解析出草稿（不花 token）
 * 2. 内容像 **公式表达式** → 构造最小 Schema 草稿（不花 token）
 * 3. 其余 → 交给 AI 解析
 *
 * ### 但 AI 入口始终可用
 *
 * 本地能解析**不等于**用户想要本地结果（比如一段 `V=pi*r^2*h`
 * 也可能希望 AI 补全参数表与单位）。所以主按钮永远是
 * 「用 AI 解析并生成」；本地解析成功时**额外**出现一个
 * 「直接导入」次级入口，而不是把 AI 路径藏起来。
 *
 * ## 🔴 事件要先订阅、再发命令
 *
 * 事件载荷是**增量**，命令的 Promise 要等生成结束才 resolve。
 * 先 `await` 命令再订阅，开头的思考内容就永久丢了。
 * 顺序固定：`subscribeAi()` → `normalizeFromQuery()`。
 *
 * ## 结果不直接落库
 *
 * AI / 本地解析都可能漏变量、写错单位。先给**预览**，
 * 用户确认「打开工作台」才校验 + 保存。
 */
import { computed, onActivated, onBeforeUnmount, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { aiApi, evalApi, formulaApi, historyApi } from '@/api'
import { normalizeError } from '@/api/invoke'
import ImageAttachRow from '@/components/image/ImageAttachRow.vue'
import ThinkingBox from '@/components/llm/ThinkingBox.vue'
import MarkdownText from '@/components/llm/MarkdownText.vue'
import MathDisplay from '@/components/math/MathDisplay.vue'
import SourceBadge from '@/components/common/SourceBadge.vue'
import InputHistoryButton from '@/components/common/InputHistoryButton.vue'
import { errorMessage, isCancelled, type CommandError } from '@/types/error'
import { useInputHistory, type InputHistoryItem } from '@/composables/useInputHistory'
import { useActiveModel } from '@/composables/useActiveModel'
import { PREF_KEYS } from '@/types/system'
import { formatTokens } from '@/utils/format'
import { useThinkingStore } from '@/stores/thinking'
import type { NormalizeResult } from '@/types/ai'
import type { FormulaSchema } from '@/types/domain'

const route = useRoute()
const router = useRouter()
/** 刚生成出来的思考（需求 4：公式页要能看到它） */
const thinkingStore = useThinkingStore()

/** 输入内容（公式 / JSON / 自然语言都行） */
const text = ref('')
/** 附图 id（顺序即附图顺序） */
const imageIds = ref<string[]>([])

const running = ref(false)
const streaming = ref(false)
const thinking = ref('')
/** 已收到的正文段数（只做进度提示；正文是 JSON，不直接展示） */
const chunkCount = ref(0)
const startedAt = ref<number | null>(null)
const error = ref<CommandError | null>(null)

/** AI 解析结果 */
const aiResult = ref<NormalizeResult | null>(null)
/** 本地解析出的草稿（有值时显示「直接导入」） */
const localDraft = ref<FormulaSchema | null>(null)

/** 当前活跃档位名（让人知道这次是谁在算）。共享状态，见 useActiveModel */
const { activeLabel, refreshActiveModel } = useActiveModel()

// ---------------------------------------------------------------- 初始化

/**
 * 本页被 `keep-alive` 缓存 —— 从设置页改完模型切回来时
 * `onMounted` 不会再跑，所以这里用 `onActivated` 兜底刷新一次
 * （设置页也会主动推送，这里是双保险）。
 */
onActivated(() => {
  void refreshActiveModel(true)
})

/**
 * 顶栏快捷搜索的兜底项会把关键词带过来（`/query?q=…`）。
 *
 * ⚠️ 必须用 `watch` 而不是只在 `onMounted` 里读一次：
 * 本页被 `keep-alive` 缓存后**不会重新挂载**，
 * 第二次从顶栏跳过来时 `onMounted` 不会再执行，关键词就丢了。
 *
 * 🔴 **已有输入时不直接覆盖**（需求 1：防止被覆盖）。
 * 用户可能正在写需求，这时从顶栏跳过来会把已写的内容**静默冲掉** ——
 * 先问一次，他可以选择保留当前内容。
 */
watch(
  () => route.query.q,
  async (q) => {
    const v = Array.isArray(q) ? q[0] : q
    if (typeof v !== 'string' || !v.trim()) return
    const next = v.trim()

    if (text.value.trim() && text.value.trim() !== next) {
      try {
        await ElMessageBox.confirm(
          `当前输入框里已有内容，是否用顶栏搜索的关键词「${next}」替换？`,
          '替换当前输入？',
          { confirmButtonText: '替换', cancelButtonText: '保留当前', type: 'warning' },
        )
      } catch {
        return // 用户选择保留
      }
    }

    text.value = next
    localDraft.value = null
    aiResult.value = null
  },
  { immediate: true },
)

void refreshActiveModel()

// ---------------------------------------------------------------- 本地解析

/** 新建空 Schema（变量为空，等用户补） */
function emptySchema(expression = ''): FormulaSchema {
  return {
    id: '',
    resultName: '',
    resultSymbol: 'R',
    resultOutputs: [],
    expression,
    sourceEquations: [],
    altExpressions: [],
    constants: {},
    variables: [],
    domain: '',
    tags: [],
    imageIds: [],
    source: { kind: 'CUSTOM', verified: false },
    schemaVersion: 3,
    createdAt: 0,
    updatedAt: 0,
  }
}

/** 补全 Schema 的缺省字段（粘贴的 JSON 可能不全） */
function fillDefaults(partial: Partial<FormulaSchema>): FormulaSchema {
  const base = emptySchema()
  return {
    ...base,
    ...partial,
    // 这几项必须是数组/对象，`undefined` 会让后端反序列化失败
    variables: Array.isArray(partial.variables) ? partial.variables : [],
    constants: partial.constants ?? {},
    tags: Array.isArray(partial.tags) ? partial.tags : [],
    sourceEquations: Array.isArray(partial.sourceEquations) ? partial.sourceEquations : [],
    altExpressions: Array.isArray(partial.altExpressions) ? partial.altExpressions : [],
    resultOutputs: Array.isArray(partial.resultOutputs) ? partial.resultOutputs : [],
    imageIds: Array.isArray(partial.imageIds) ? partial.imageIds : [],
    source: partial.source ?? { kind: 'CUSTOM', verified: false },
    schemaVersion: 3,
  }
}

/** 是否像一个公式表达式（而非自然语言描述） */
function looksLikeExpression(s: string): boolean {
  const t = s.trim()
  if (!t || t.length > 300) return false
  // 含运算符且不含中文（中文更像描述）
  return /[+\-*/^()]/.test(t) && !/[\u4e00-\u9fff]/.test(t)
}

/** 是否像 Schema JSON */
function looksLikeJson(s: string): boolean {
  return s.trim().startsWith('{')
}

/**
 * 尝试本地解析。
 *
 * **不弹错**：解析失败只是「这条路走不通」，AI 那条路照样能用。
 * 所以这里静默返回 `null`，由 UI 决定要不要给提示。
 */
function tryLocalParse(): FormulaSchema | null {
  const t = text.value.trim()
  if (!t) return null

  if (looksLikeJson(t)) {
    try {
      return fillDefaults(JSON.parse(t) as Partial<FormulaSchema>)
    } catch {
      return null
    }
  }

  if (looksLikeExpression(t)) {
    const s = emptySchema(t)
    const m = /^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=/.exec(t)
    if (m) {
      s.resultSymbol = m[1]
      s.resultName = `新公式（${m[1]}）`
    } else {
      s.resultName = '新公式'
    }
    return s
  }

  return null
}

/** 输入变化时刷新本地草稿（供「直接导入」与提示条使用） */
watch(text, () => {
  localDraft.value = tryLocalParse()
})

/** 本地解析命中的类型（提示条文案用） */
const localKind = computed(() => {
  if (!localDraft.value) return ''
  return looksLikeJson(text.value) ? 'JSON 结构' : '公式表达式'
})

// ---------------------------------------------------------------- 发送

let unlisten: (() => void) | null = null

onBeforeUnmount(() => {
  unlisten?.()
  unlisten = null
})

const canSend = computed(
  () => !running.value && (text.value.trim().length > 0 || imageIds.value.length > 0),
)

// ---------------------------------------------------------------- 输入历史（需求 5）

/** 提交成功时记一条；点历史图标可把文本 + 附图一起恢复 */
const history = useInputHistory(PREF_KEYS.searchHistory)
void history.load()

/**
 * 恢复一条历史。
 *
 * 文本与附图**一起**恢复 —— 只恢复文本会让用户以为图也回来了，
 * 而附图顺序是语义（模型按「第 N 张」写 `{{img:N}}`），不能只给一半。
 */
function restoreFromHistory(it: InputHistoryItem): void {
  text.value = it.text
  imageIds.value = [...it.imageIds]
  localDraft.value = null
  aiResult.value = null
  error.value = null
  ElMessage.success('已恢复这条输入')
}

/** 交给 AI 解析（主路径） */
async function runAi(): Promise<void> {
  if (running.value) return
  if (!canSend.value) {
    ElMessage.warning('请先粘贴内容或写下需求，或至少附一张图')
    return
  }

  running.value = true
  streaming.value = true
  error.value = null
  aiResult.value = null
  thinking.value = ''
  chunkCount.value = 0
  startedAt.value = Date.now()

  /** 事件里带回的错误（比 Promise 的 rejection 信息更全，含 aiCode） */
  let eventError: CommandError | null = null

  unlisten?.()
  unlisten = await aiApi.subscribeAi({
    onThinking: (t) => {
      thinking.value += t
    },
    onChunk: () => {
      chunkCount.value += 1
    },
    onDone: () => {
      streaming.value = false
    },
    onError: (e) => {
      eventError = e
      streaming.value = false
    },
  })

  try {
    const r = await aiApi.normalizeFromQuery(text.value.trim(), imageIds.value)
    aiResult.value = r
    // 提交成功才记历史（需求 5）——不是每次击键，否则历史会被半截输入塞满
    void history.push(text.value, imageIds.value)
    ElMessage.success('解析完成，请核对后打开工作台')
  } catch (e) {
    const err = normalizeError(e)
    if (isCancelled(err)) {
      // 用户自己点的取消：静默收尾，不弹错误框
      ElMessage.info('已取消')
      return
    }
    error.value = eventError ?? err
  } finally {
    streaming.value = false
    running.value = false
    unlisten?.()
    unlisten = null
  }
}

async function cancel(): Promise<void> {
  try {
    await aiApi.aiCancel()
  } catch {
    // 取消本身失败不打扰用户；后端也会在下次任务开始时清标志
  }
}

function reset(): void {
  text.value = ''
  imageIds.value = []
  thinking.value = ''
  aiResult.value = null
  localDraft.value = null
  error.value = null
  chunkCount.value = 0
}

function discard(): void {
  aiResult.value = null
  localDraft.value = null
}

// ---------------------------------------------------------------- 结果

/** 当前要展示/落库的 schema：AI 优先，其次本地草稿 */
const schema = computed<FormulaSchema | null>(
  () => aiResult.value?.schema ?? localDraft.value ?? null,
)

const isAiResult = computed(() => Boolean(aiResult.value?.schema))

const usageText = computed(() => {
  const u = aiResult.value?.usage
  if (!u) return ''
  return `本次合计 ${formatTokens(u.totalTokens)} tokens（输入 ${formatTokens(u.promptTokens)} · 输出 ${formatTokens(u.completionTokens)}）`
})

/**
 * 思考框上显示的 token 数（需求 1）。
 *
 * - 生成中：只能按「思考文本长度 / 2」估算（与后端同一口径），标「约」
 * - 拿到厂商真实 usage 后：换成确定值
 *
 * 不假装精确 —— 估算值一定要带「约」字，否则用户会拿它对账。
 */
const thinkingTokens = computed(() => Math.ceil(thinking.value.length / 2))
const boxTokens = computed(() => aiResult.value?.usage?.totalTokens ?? thinkingTokens.value)
const boxTokensLive = computed(() => !aiResult.value?.usage)

const saving = ref(false)

/** 校验 + 落库 + 打开工作台 */
async function openInWorkspace(): Promise<void> {
  const s = schema.value
  if (!s || saving.value) return

  // 本地草稿带上附图（AI 结果里后端已写好 imageIds）
  const withImages: FormulaSchema = isAiResult.value
    ? s
    : { ...s, imageIds: imageIds.value.length > 0 ? [...imageIds.value] : s.imageIds }

  saving.value = true
  try {
    await evalApi.validateSchemaCmd(withImages)
  } catch (e) {
    ElMessage.error(`校验未通过：${errorMessage(normalizeError(e))}`)
    saving.value = false
    return
  }

  try {
    const toSave: FormulaSchema = {
      ...withImages,
      // 兜底：模型偶尔会漏 id（契约要求 usr: 前缀）
      id: withImages.id?.trim() ? withImages.id : `usr:${Date.now().toString(36)}`,
      updatedAt: Date.now(),
      createdAt: withImages.createdAt || Date.now(),
    }
    await formulaApi.formulaSave(toSave)
    // 需求 4：把这次的思考挂到新公式上，公式页立刻就能看到
    thinkingStore.remember(toSave.id, thinking.value)
    // 需求 7：新建公式要**立刻**在「历史」里留一条（无输入无结果）。
    // 失败不阻断 —— 历史记不上不该让用户保存不了公式。
    try {
      await historyApi.historyRecordCreated(toSave.id, thinking.value)
    } catch (e) {
      console.warn('[QueryView] 记「公式创建」历史失败', e)
    }
    ElMessage.success('已保存')
    router.push({ name: 'formula', params: { id: toSave.id } })
  } catch (e) {
    ElMessage.error(errorMessage(normalizeError(e)))
  } finally {
    saving.value = false
  }
}

/** 重试：保留输入，只清错误 */
function retry(): void {
  error.value = null
  void runAi()
}

// ---------------------------------------------------------------- 离开

/**
 * 离开本页**什么都不做**（需求 1 / 6）。
 *
 * ## 🔴 这里曾经会 `cancel()` —— 那是「切标签就中断 AI」的根因
 *
 * 原来的写法是「正在生成 → 先取消再离开」。当时以为「后台任务继续跑、
 * 回来状态会错乱」，但实际后果严重得多：
 *
 * - 用户切去看一眼公式库，**AI 生成就被掐断**，token 花了、结果没了
 * - 本页被 `keep-alive` 缓存，组件与事件订阅**都还活着** ——
 *   根本不存在「状态错乱」，只有「被主动打断」
 *
 * 现在**不拦、不取消**：切走之后流式事件照常累加到本页的 ref 上，
 * 切回来就能看到已经生成好的结果。
 *
 * ## 需要用户主动取消时
 *
 * 界面上有「取消」按钮（`cancel()`）。生成中途想放弃，那是明确动作。
 *
 * ## 覆盖确认不在这里
 *
 * 「从公式库/历史/收藏/新公式跳进公式计算会不会顶掉正在用的公式」
 * 放在路由守卫里（`router/index.ts`）—— 要同时覆盖四个入口，
 * 写在这一页只能管住自己。
 */
</script>

<template>
  <div class="view">
    <!-- ① 输入 -->
    <section class="panel">
      <div class="panel__head">
        <span class="panel__title">新建公式</span>
        <span v-if="activeLabel" class="panel__model">{{ activeLabel }}</span>
      </div>

      <div class="panel__body">
        <el-input
          v-model="text"
          type="textarea"
          :rows="6"
          resize="vertical"
          :disabled="running"
          placeholder="粘贴公式或 JSON，或直接用日常语言说清要算什么。例如：一根 3 米长的悬臂梁，端部作用 10kN 集中力，求最大弯矩和挠度"
        />

        <!-- 本地解析命中：给一条不花 token 的捷径，但不抢主按钮 -->
        <div v-if="localDraft && !aiResult" class="detect">
          <span class="detect__text">识别到{{ localKind }}，可直接导入（不调用 AI）</span>
          <button
            class="detect__go"
            type="button"
            :disabled="running || saving"
            @click="openInWorkspace()"
          >
            直接导入
          </button>
        </div>

        <div class="attach">
          <span class="attach__label">附图（可选，最多 5 张）</span>
          <ImageAttachRow v-model="imageIds" :max="5" :disabled="running" />
        </div>

        <div class="actions">
          <el-button v-if="!running" type="primary" :disabled="!canSend" @click="runAi()">
            用 AI 解析并生成
          </el-button>
          <el-button v-else @click="cancel()">取消</el-button>
          <el-button v-if="text || imageIds.length" :disabled="running" @click="reset()">
            清空
          </el-button>
          <InputHistoryButton
            :items="history.items.value"
            :disabled="running"
            title="新建公式 · 输入历史"
            hint="从历史里选一条恢复（文本 + 附图）"
            @restore="restoreFromHistory"
            @remove="(i: number) => history.removeAt(i)"
            @clear="history.clear()"
          />
          <span class="actions__hint">
            图片按附图顺序送给模型，详解里的「附图 N」与这里一致
          </span>
        </div>
      </div>
    </section>

    <!-- ② 思考过程（流式；结束后自动折叠，需求 1） -->
    <ThinkingBox
      :text="thinking"
      :streaming="streaming"
      :started-at="startedAt"
      :tokens="boxTokens"
      :tokens-live="boxTokensLive"
      title="AI 思考过程"
      sticky
    />

    <!-- ③ 生成中的进度（还没有思考内容时给个交代） -->
    <section v-if="running && !thinking" class="panel">
      <div class="panel__body progress">
        <el-skeleton :rows="3" animated />
        <p class="progress__text">
          模型正在生成公式结构…{{ chunkCount > 0 ? `（已接收 ${chunkCount} 段）` : '' }}
        </p>
      </div>
    </section>

    <!-- ④ 错误 -->
    <section v-if="error" class="panel panel--error">
      <div class="panel__body">
        <h3 class="error__title">解析失败</h3>
        <p class="error__msg">{{ errorMessage(error) }}</p>
        <p v-if="error.kind === 'aiError'" class="error__code">错误码：{{ error.aiCode }}</p>
        <div class="actions actions--end">
          <el-button @click="router.push({ name: 'settings' })">去设置</el-button>
          <el-button type="primary" @click="retry()">重试</el-button>
        </div>
      </div>
    </section>

    <!-- ⑤ 结果预览 -->
    <section v-if="schema" class="panel">
      <div class="panel__head">
        <span class="panel__title">{{ isAiResult ? 'AI 解析结果' : '待导入的公式' }}</span>
        <SourceBadge :kind="schema.source.kind" :verified="schema.source.verified" />
      </div>

      <div class="panel__body">
        <div class="preview">
          <div class="preview__row">
            <span class="preview__key">名称</span>
            <span class="preview__val">{{ schema.resultName || '（未命名）' }}</span>
          </div>
          <div class="preview__row">
            <span class="preview__key">领域</span>
            <span class="preview__val">{{ schema.domain || '—' }}</span>
          </div>
          <div class="preview__row">
            <span class="preview__key">参数</span>
            <span class="preview__val">{{ schema.variables.length }} 个</span>
          </div>
          <div class="preview__row">
            <span class="preview__key">分步</span>
            <span class="preview__val">{{ schema.stepsTemplate?.length ?? 0 }} 步</span>
          </div>
        </div>

        <div v-if="schema.expression" class="expr">
          <MathDisplay :expression="schema.expression" :constants="schema.constants" />
        </div>

        <div v-if="schema.designNotes" class="notes">
          <h4 class="notes__label">设计说明</h4>
          <MarkdownText :text="schema.designNotes" />
        </div>

        <p v-if="usageText" class="usage">{{ usageText }}</p>

        <div class="actions actions--end">
          <el-button :disabled="saving" @click="discard()">丢弃</el-button>
          <el-button type="primary" :loading="saving" @click="openInWorkspace()">
            打开工作台
          </el-button>
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.view {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  /* 铺满窗口：需求「窗口缩放要自适应」——不再限宽 */
  max-width: none;
}

.panel {
  border-radius: var(--r-card);
  background: var(--c-surface);
}

.panel--error {
  border-left: 3px solid var(--c-danger);
}

.panel__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--sp-3);
  padding: var(--sp-4) var(--sp-4) 0;
}

.panel__title {
  font-weight: 600;
  color: var(--c-text);
}

.panel__model {
  padding: 1px var(--sp-2);
  border-radius: var(--r-pill);
  background: var(--c-surface-2);
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.panel__body {
  padding: var(--sp-3) var(--sp-4) var(--sp-4);
}

/* 本地解析命中提示条 */
.detect {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
  margin-top: var(--sp-2);
  padding: var(--sp-2) var(--sp-3);
  border-radius: var(--r-sm);
  background: var(--c-primary-light);
}

.detect__text {
  color: var(--c-primary);
  font-size: var(--f-size-sm);
}

.detect__go {
  margin-left: auto;
  padding: 2px var(--sp-3);
  border: var(--hairline) solid var(--c-primary);
  border-radius: var(--r-btn);
  background: transparent;
  color: var(--c-primary);
  font-family: inherit;
  font-size: var(--f-size-sm);
  cursor: pointer;
}

.detect__go:hover:not(:disabled) {
  background: var(--c-surface);
}

.detect__go:disabled {
  opacity: 0.6;
  cursor: default;
}

.attach {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  margin-top: var(--sp-3);
}

.attach__label {
  font-size: var(--f-size-sm);
  color: var(--c-text-2);
}

.actions {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  margin-top: var(--sp-3);
}

.actions--end {
  justify-content: flex-end;
}

.actions__hint {
  margin-left: auto;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.progress {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
}

.progress__text {
  margin: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
}

.error__title {
  margin: 0 0 var(--sp-2);
  font-size: var(--f-size-lg);
  color: var(--c-text);
}

.error__msg {
  margin: 0;
  color: var(--c-text-2);
  font-size: var(--f-size-sm);
  line-height: 1.7;
}

.error__code {
  margin: var(--sp-2) 0 0;
  color: var(--c-text-3);
  font-family: var(--f-mono);
  font-size: var(--f-size-xs);
}

.preview {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
  gap: var(--sp-2) var(--sp-4);
}

.preview__row {
  display: flex;
  gap: var(--sp-2);
  min-width: 0;
}

.preview__key {
  flex-shrink: 0;
  width: 48px;
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
}

.preview__val {
  color: var(--c-text);
  font-size: var(--f-size-sm);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.expr {
  margin-top: var(--sp-3);
  padding: var(--sp-3);
  border-radius: var(--r-sm);
  background: var(--c-surface-2);
}

.notes {
  margin-top: var(--sp-3);
}

.notes__label {
  margin: 0 0 var(--sp-1);
  font-size: var(--f-size-sm);
  font-weight: 600;
  color: var(--c-text-2);
}

.usage {
  margin: var(--sp-3) 0 0;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}
</style>
