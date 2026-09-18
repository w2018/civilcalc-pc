<script setup lang="ts">
/**
 * RefineDialog —— AI 公式微调（基于既有公式按需求改写）。
 *
 * 后端命令 `refine_formula` 见 `commands/ai.rs`。
 *
 * ## 微调的结果是**新公式**，不是就地改
 *
 * 后端会做两件事：
 *
 * 1. **名称版本化**：`圆柱体积` → `圆柱体积-v1.0`（连续微调递增），
 *    由 `civilcalc_llm::next_version_name` 生成
 * 2. 把 `revised_from_id` 记成原公式 id，形成派生链
 *
 * 所以这里保存时**不带原 id**（带 `usr:` 兜底），落到库里是一条新记录，
 * 原公式原样保留。用户不满意可以删掉新公式，不会丢东西。
 *
 * ## 🔴 支持附图（需求 2）
 *
 * 「这张图里标注的做法改成……」「按图纸第 3 页的尺寸补一个参数」——
 * 这类需求光靠文字说不清，必须能带图。
 *
 * 附图顺序是**语义**：父公式自身的附图由后端自动前置，这里新增的排在后面。
 * 模型按「第 N 张」写 `{{img:N}}`，顺序错了详解里的图会指错。
 * 所以 `ImageAttachRow` 上的序号不是装饰。
 *
 * ## 两栏布局（需求 2：重写界面）
 *
 * 原来是一条竖排的窄弹窗（720px）：输入、预设、按钮、思考、结果全挤在一起，
 * 生成时页面会**跳来跳去**（思考框一出现，结果就被顶下去）。
 *
 * 现在左右分栏、各自独立滚动：
 *
 * ```
 * ┌──────────────┬──────────────────────────┐
 * │ 怎么写        │ 改成了什么                │
 * │  需求输入     │  结果预览 / 空态          │
 * │  预设 chip    │  （表达式 / 参数 / 说明）  │
 * │  附图         │                          │
 * │  操作按钮     │                          │
 * │  思考过程     │                          │
 * └──────────────┴──────────────────────────┘
 * ```
 *
 * 左栏是「我给的」，右栏是「它给的」—— 生成过程中右边从空态变成结果，
 * 左边完全不动，视线不用追着跳。
 *
 * ## 与「生成详解」一样不流式落库
 *
 * 微调走的是 `normalize` 那条链路（**流式**），所以有思考过程可看；
 * 但结果仍然要用户确认后才保存 —— 模型可能把公式改坏。
 *
 * ## 🔴 先订阅再发命令
 *
 * 与 `QueryView` 同一条铁律：事件载荷是增量，命令的 Promise
 * 要等生成结束才 resolve。顺序反了会丢掉开头的思考内容。
 */
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { aiApi, evalApi, formulaApi, historyApi } from '@/api'
import { normalizeError } from '@/api/invoke'
import ThinkingBox from './ThinkingBox.vue'
import MathDisplay from '@/components/math/MathDisplay.vue'
import MarkdownText from '@/components/llm/MarkdownText.vue'
import ImageAttachRow from '@/components/image/ImageAttachRow.vue'
import InputHistoryButton from '@/components/common/InputHistoryButton.vue'
import { errorMessage, isCancelled, type CommandError } from '@/types/error'
import type { NormalizeResult } from '@/types/ai'
import { useInputHistory, type InputHistoryItem } from '@/composables/useInputHistory'
import { useThinkingStore } from '@/stores/thinking'
import { PREF_KEYS } from '@/types/system'
import { formatTokens } from '@/utils/format'
import type { FormulaSchema } from '@/types/domain'

const props = defineProps<{
  /** 是否显示（`v-model`） */
  modelValue: boolean
  /** 被微调的公式 */
  schema: FormulaSchema | null
}>()

const emit = defineEmits<{
  (e: 'update:modelValue', v: boolean): void
  /** 微调结果已保存为新公式 */
  (e: 'saved', id: string): void
}>()

/** 微调需求 */
const requirement = ref('')
/** 本次新增的附图 id（父公式的图由后端自动前置） */
const imageIds = ref<string[]>([])
const running = ref(false)
const streaming = ref(false)
const thinking = ref('')
const startedAt = ref<number | null>(null)
const error = ref<CommandError | null>(null)
const result = ref<NormalizeResult | null>(null)
const saving = ref(false)

/** 刚生成出来的思考要挂到新公式上（需求 4） */
const thinkingStore = useThinkingStore()

/** 每次打开都重置（避免上一次的残留串到这一次） */
watch(
  () => props.modelValue,
  (open) => {
    if (!open) return
    requirement.value = ''
    imageIds.value = []
    thinking.value = ''
    result.value = null
    error.value = null
    startedAt.value = null
  },
)

/** 微调需求的历史（换公式也保留 —— 需求写法往往可复用） */
const history = useInputHistory(PREF_KEYS.refineHistory)
void history.load()

/** 恢复一条历史：文本与附图一起（只给一半会让用户以为图也回来了） */
function restoreFromHistory(it: InputHistoryItem): void {
  requirement.value = it.text
  imageIds.value = [...it.imageIds]
  ElMessage.success('已恢复这条需求')
}

let unlisten: (() => void) | null = null

onBeforeUnmount(() => {
  unlisten?.()
  unlisten = null
})

const refined = computed<FormulaSchema | null>(() => result.value?.schema ?? null)

/** 常用微调意图，一键填入（降低「不知道该怎么写」的门槛） */
const PRESETS = [
  '单位改成 kN / m，并保留两位小数',
  '补充一个中间量：先算截面面积再算体积',
  '把结果拆成「总重」和「单位长度重」两个输出',
  '增加一个可选参数，并给出默认值',
] as const

/** 左栏是否处于「已生成过」状态（决定按钮文案） */
const hasResult = computed(() => refined.value !== null)

/** 思考 token：生成中按文本估算，结束后用厂商真实值 */
const thinkingTokens = computed(() => Math.ceil(thinking.value.length / 2))
const boxTokens = computed(() => result.value?.usage?.totalTokens ?? thinkingTokens.value)
const boxTokensLive = computed(() => !result.value?.usage)

const usageText = computed(() => {
  const u = result.value?.usage
  if (!u) return ''
  return `本次合计 ${formatTokens(u.totalTokens)} tokens（输入 ${formatTokens(u.promptTokens)} · 输出 ${formatTokens(u.completionTokens)}）`
})

async function run(): Promise<void> {
  const s = props.schema
  if (!s || running.value) return
  if (!requirement.value.trim() && imageIds.value.length === 0) {
    ElMessage.warning('请先写清要怎么改，或至少附一张图')
    return
  }

  running.value = true
  streaming.value = true
  error.value = null
  result.value = null
  thinking.value = ''
  startedAt.value = Date.now()

  /** 事件里带回的错误（比 Promise 的 rejection 信息更全，含 aiCode） */
  let eventError: CommandError | null = null
  /** 发送前固定下来 —— 生成结束后 `imageIds` 可能已被改动 */
  const sentImages = [...imageIds.value]
  const sentText = requirement.value.trim()

  unlisten?.()
  unlisten = await aiApi.subscribeAi({
    onThinking: (t) => {
      thinking.value += t
    },
    onChunk: () => {},
    onDone: () => {
      streaming.value = false
    },
    onError: (e) => {
      eventError = e
      streaming.value = false
    },
  })

  try {
    result.value = await aiApi.refineFormula(s, sentText, sentImages)
    // 需求 5：成功才记历史（失败/取消不记，否则历史里全是半截需求）
    void history.push(sentText, sentImages)
    ElMessage.success('微调完成，请核对后保存')
  } catch (e) {
    const err = normalizeError(e)
    if (isCancelled(err)) {
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
    // 取消失败不打扰用户
  }
}

/**
 * 关闭前的守卫。
 *
 * 微调是**流式**的：弹窗关掉并不会让后端停下，结果会「悄悄」写进
 * `result`，而下次打开时 `watch(modelValue)` 会把它重置掉 ——
 * 用户看到的是「生成到一半，关了再打开，什么都没了」。
 *
 * 所以正在生成时**先问一次**（关闭 = 明确放弃，与「切标签页不该中断」
 * 是两件事：切标签页组件还活着、结果还在；关弹窗会重置状态）。
 */
async function beforeClose(done: () => void): Promise<void> {
  if (!running.value) {
    done()
    return
  }
  try {
    await ElMessageBox.confirm('正在微调，关闭会取消这次生成。', '确认关闭？', {
      type: 'warning',
      confirmButtonText: '关闭并取消',
      cancelButtonText: '继续等待',
    })
  } catch {
    return // 继续等待
  }
  await cancel()
  done()
}

/**
 * 页脚「关闭」按钮也走同一道守卫。
 *
 * `before-close` 只覆盖标题栏 ✕ / ESC / 点遮罩 —— 自己写的按钮要显式接上，
 * 否则从按钮关掉就没有提示。
 */
async function requestClose(): Promise<void> {
  await beforeClose(() => emit('update:modelValue', false))
}

/** 保存为新公式（原公式不动） */
async function saveAsNew(): Promise<void> {
  const s = refined.value
  if (!s || saving.value) return

  saving.value = true
  try {
    await evalApi.validateSchemaCmd(s)
  } catch (e) {
    ElMessage.error(`校验未通过：${errorMessage(normalizeError(e))}`)
    saving.value = false
    return
  }

  try {
    const toSave: FormulaSchema = {
      ...s,
      id: s.id?.trim() ? s.id : `usr:${Date.now().toString(36)}`,
      updatedAt: Date.now(),
      createdAt: s.createdAt || Date.now(),
    }
    await formulaApi.formulaSave(toSave)
    // 需求 4：把这次的思考挂到新公式上，打开后立刻能看到
    thinkingStore.remember(toSave.id, thinking.value)
    // 需求 7：微调出的新公式也要立刻在「历史」里留一条（无输入无结果）
    try {
      await historyApi.historyRecordCreated(toSave.id, thinking.value)
    } catch (e) {
      console.warn('[RefineDialog] 记「公式创建」历史失败', e)
    }
    ElMessage.success('已保存为新公式')
    emit('saved', toSave.id)
    emit('update:modelValue', false)
  } catch (e) {
    ElMessage.error(errorMessage(normalizeError(e)))
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <el-dialog
    :model-value="modelValue"
    title="AI 微调公式"
    width="920px"
    top="6vh"
    :close-on-click-modal="false"
    :before-close="beforeClose"
    class="refine-dialog"
    @update:model-value="(v: boolean) => emit('update:modelValue', v)"
  >
    <div class="rf">
      <!-- ================= 左：我给的 ================= -->
      <div class="rf__col rf__col--input">
        <div class="rf__src">
          <span class="rf__src-label">基于</span>
          <span class="rf__src-name">{{ schema?.resultName || '当前公式' }}</span>
        </div>
        <p class="rf__tip">
          结果会存成**一条新公式**（名称自动加 `-v1.0`），原公式不受影响。
        </p>

        <label class="rf__field">
          <span class="rf__label">要怎么改？</span>
          <el-input
            v-model="requirement"
            type="textarea"
            :rows="4"
            resize="vertical"
            :disabled="running"
            placeholder="例如：单位改成 kN / m，并补充一个中间量「截面面积」"
            @keydown.enter.exact.prevent="run()"
          />
        </label>

        <div class="rf__presets">
          <button
            v-for="p in PRESETS"
            :key="p"
            class="preset"
            type="button"
            :disabled="running"
            @click="requirement = p"
          >
            {{ p }}
          </button>
        </div>

        <div class="rf__attach">
          <div class="rf__attach-head">
            <span class="rf__label">附图（可选）</span>
            <InputHistoryButton
              :items="history.items.value"
              :disabled="running"
              title="公式微调 · 需求历史"
              hint="从历史里选一条恢复（文本 + 附图）"
              @restore="restoreFromHistory"
              @remove="(i: number) => history.removeAt(i)"
              @clear="history.clear()"
            />
          </div>
          <ImageAttachRow v-model="imageIds" :max="5" :disabled="running" compact />
        </div>

        <div class="rf__ops">
          <el-button v-if="!running" type="primary" @click="run()">
            {{ hasResult ? '重新微调' : '开始微调' }}
          </el-button>
          <el-button v-else @click="cancel()">取消</el-button>
          <span class="rf__ops-hint">回车即开始，多行用 Ctrl + 回车</span>
        </div>

        <ThinkingBox
          :text="thinking"
          :streaming="streaming"
          :started-at="startedAt"
          :tokens="boxTokens"
          :tokens-live="boxTokensLive"
          title="AI 思考过程"
        />

        <div v-if="error" class="rf__err">
          <p class="rf__err-text">{{ errorMessage(error) }}</p>
        </div>
      </div>

      <!-- ================= 右：它给的 ================= -->
      <div class="rf__col rf__col--result">
        <div v-if="!refined" class="rf__empty">
          <template v-if="running">
            <el-skeleton :rows="5" animated />
            <p class="rf__empty-text">模型正在按你的要求改写公式…</p>
          </template>
          <template v-else>
            <p class="rf__empty-title">改完的结果会出现在这里</p>
            <p class="rf__empty-text">
              左侧写清要怎么改（可以附图），点「开始微调」。
              结果只是预览，确认没问题再点「保存为新公式」。
            </p>
          </template>
        </div>

        <template v-else>
          <div class="rf__head">
            <span class="rf__name">{{ refined.resultName }}</span>
            <span class="rf__meta">{{ refined.variables.length }} 个参数</span>
          </div>

          <div v-if="refined.expression" class="rf__expr">
            <MathDisplay :expression="refined.expression" :constants="refined.constants" size="sm" />
          </div>

          <div v-if="refined.variables.length > 0" class="rf__vars">
            <span v-for="v in refined.variables" :key="v.symbol" class="rf__var">
              {{ v.symbol }}
              <span v-if="v.unit" class="rf__var-unit">{{ v.unit }}</span>
            </span>
          </div>

          <div v-if="refined.designNotes" class="rf__notes">
            <h4 class="rf__notes-label">设计说明</h4>
            <MarkdownText :text="refined.designNotes" />
          </div>

          <p v-if="usageText" class="rf__usage">{{ usageText }}</p>
        </template>
      </div>
    </div>

    <template #footer>
      <el-button @click="requestClose()">关闭</el-button>
      <el-button type="primary" :disabled="!refined" :loading="saving" @click="saveAsNew()">
        保存为新公式
      </el-button>
    </template>
  </el-dialog>
</template>

<style scoped>
/* 两栏各自滚动：生成时左栏完全不动，视线不用追着跳 */
.rf {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  /* 🔴 行高必须显式写成 `minmax(0, 1fr)` 并给容器 `overflow: hidden`。
     否则网格项默认 `min-height: auto` —— 思考过程越长，行就越高，
     把整个弹窗顶出窗口（需求 5：思考过程会顶出窗口）。 */
  grid-template-rows: minmax(0, 1fr);
  gap: var(--sp-4);
  max-height: 64vh;
  overflow: hidden;
}

/* 窄窗口（<900px）退回单栏：两栏会窄到看不清 */
@media (max-width: 900px) {
  .rf {
    grid-template-columns: minmax(0, 1fr);
    grid-template-rows: none;
    max-height: none;
    overflow: visible;
  }
}

.rf__col {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  min-width: 0;
  /* `min-height: 0` 是让 `overflow-y` 真正生效的前提（flex/grid 项默认不收缩） */
  min-height: 0;
  overflow-y: auto;
  padding-right: var(--sp-1);
}

/**
 * 弹窗里的思考过程用**更小的字号 + 更矮的高度**（需求 5）。
 *
 * 它是「过程」不是「结果」，不该占掉半屏；而且这一栏本来就要
 * 同时容纳输入框、预设、附图与按钮。
 */
.rf__col--input :deep(.think__body) {
  max-height: 132px;
  padding: var(--sp-2);
  font-size: var(--f-size-xs);
  line-height: 1.6;
}

.rf__col--input :deep(.think__head) {
  padding: var(--sp-2) var(--sp-3);
  font-size: var(--f-size-xs);
}

.rf__col--input :deep(.think__wrap) {
  padding: 0 var(--sp-3) var(--sp-2);
}

.rf__col--result {
  padding-left: var(--sp-4);
  border-left: var(--hairline) solid var(--c-divider);
}

@media (max-width: 900px) {
  .rf__col--result {
    padding-left: 0;
    border-left: none;
    border-top: var(--hairline) solid var(--c-divider);
    padding-top: var(--sp-3);
  }
}

.rf__src {
  display: flex;
  align-items: baseline;
  gap: var(--sp-2);
  min-width: 0;
}

.rf__src-label {
  flex-shrink: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.rf__src-name {
  min-width: 0;
  color: var(--c-text);
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.rf__tip {
  margin: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
  line-height: 1.7;
}

.rf__field {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
}

.rf__label {
  color: var(--c-text-2);
  font-size: var(--f-size-sm);
}

.rf__presets {
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-2);
}

.preset {
  padding: 2px var(--sp-3);
  border: var(--hairline) solid var(--c-divider);
  border-radius: var(--r-pill);
  background: var(--c-surface-2);
  color: var(--c-text-2);
  font-family: inherit;
  font-size: var(--f-size-xs);
  cursor: pointer;
}

.preset:hover:not(:disabled) {
  border-color: var(--c-primary);
  color: var(--c-primary);
}

.preset:disabled {
  opacity: 0.6;
  cursor: default;
}

.rf__attach {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  padding: var(--sp-3);
  border-radius: var(--r-card);
  background: var(--c-surface-2);
}

.rf__attach-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--sp-2);
}

.rf__ops {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
}

.rf__ops-hint {
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.rf__err {
  padding: var(--sp-2) var(--sp-3);
  border-radius: var(--r-sm);
  background: var(--c-surface-2);
}

.rf__err-text {
  margin: 0;
  color: var(--c-danger);
  font-size: var(--f-size-sm);
  line-height: 1.7;
}

/* ---- 右栏 ---- */

.rf__empty {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  padding: var(--sp-6) var(--sp-4);
  text-align: center;
}

.rf__empty-title {
  margin: 0;
  color: var(--c-text-2);
  font-size: var(--f-size-base);
}

.rf__empty-text {
  margin: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
  line-height: 1.8;
}

.rf__head {
  display: flex;
  align-items: baseline;
  gap: var(--sp-2);
  min-width: 0;
}

.rf__name {
  min-width: 0;
  color: var(--c-text);
  font-size: var(--f-size-lg);
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.rf__meta {
  flex-shrink: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.rf__expr {
  padding: var(--sp-3);
  border-radius: var(--r-card);
  background: var(--c-surface-2);
  overflow-x: auto;
}

.rf__vars {
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-2);
}

.rf__var {
  padding: 1px var(--sp-2);
  border-radius: var(--r-pill);
  background: var(--c-surface-2);
  color: var(--c-text-2);
  font-family: var(--f-mono);
  font-size: var(--f-size-xs);
}

.rf__var-unit {
  color: var(--c-text-3);
}

.rf__notes-label {
  margin: 0 0 var(--sp-1);
  color: var(--c-text-2);
  font-size: var(--f-size-sm);
  font-weight: 600;
}

.rf__notes {
  min-width: 0;
}

.rf__usage {
  margin: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}
</style>
