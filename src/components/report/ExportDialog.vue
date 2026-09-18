<script setup lang="ts">
/**
 * 导出计算书对话框（P3-14）。
 *
 * 见 docs/08-IPC契约.md 组 13 与 docs/04-数据契约.md §5。
 *
 * ## 组成
 *
 * 1. **章节勾选**（7 项）→ 绑定 `ExportOptions`，持久化到 `AppConfig`
 * 2. **免责声明**文本框（空 = 用内置默认）
 * 3. **HTML 预览**（ADR-018，内建「仅供参考」标注）
 * 4. **保存位置**（四档，见 `ExportLocationPicker`）
 * 5. **导出** → 进度条（`export://progress`）→ 成功后可「打开文件 / 打开所在文件夹」
 *
 * ## 🔴 几个硬约束
 *
 * - **必须用返回值的 `path`** 打开文件：同名冲突时后端加序号
 *   （`报告(2).docx`），用户选的路径可能不存在。
 * - **预览与导出是两套渲染**，视觉可能不一致 —— 预览页已标注。
 * - **进度事件必须 unlisten**：监听在 `onExport` 里注册，`finally` 里卸载，
 *   否则每次导出都泄漏一个监听器。
 * - **空选择也能导出**（只有封面的空文档），但给个提示，不让用户误以为坏了。
 */
import { ref, watch } from 'vue'
import { ElMessage } from 'element-plus'
import {
  exportOptionsGet,
  exportOptionsSave,
  reportExportDocx,
  reportPreview,
  reportReveal,
} from '@/api/report'
import { openFile } from '@/api/opener'
import { onTauriEvent } from '@/api/events'
import { toNumericInputs } from '@/utils/inputs'
import { errorMessage } from '@/types/error'
import type { FormulaSchema } from '@/types/domain'
import {
  DEFAULT_EXPORT_OPTIONS,
  EXPORT_PROGRESS_EVENT,
  humanSize,
  isEmptySelection,
  progressPercent,
  progressStageLabel,
  type DocumentExportResult,
  type DocumentOutputTarget,
  type ExportOptions,
  type ExportProgress,
} from '@/types/report'
import ReportPreview from './ReportPreview.vue'
import ExportLocationPicker from './ExportLocationPicker.vue'

const props = defineProps<{
  formulaId: string
  schema: FormulaSchema | null
  paramValues: Record<string, string>
  modelValue: boolean
}>()

const emit = defineEmits<{
  (e: 'update:modelValue', v: boolean): void
}>()

const visible = ref(props.modelValue)
watch(
  () => props.modelValue,
  (v) => {
    visible.value = v
    if (v) void onOpen()
    else cleanup()
  },
)
watch(visible, (v) => emit('update:modelValue', v))

// ---- 状态 ----
const options = ref<ExportOptions>({ ...DEFAULT_EXPORT_OPTIONS })
const previewHtml = ref('')
const previewLoading = ref(false)
const target = ref<DocumentOutputTarget>({ kind: 'scopedDownloads' })
const exporting = ref(false)
const progress = ref<ExportProgress | null>(null)
const result = ref<DocumentExportResult | null>(null)
const errorMsg = ref<string | null>(null)

const CHAPTERS: { key: keyof ExportOptions; label: string }[] = [
  { key: 'showCover', label: '封面' },
  { key: 'showFormulaInfo', label: '公式信息' },
  { key: 'showParamsTable', label: '参数表' },
  { key: 'showResultBlock', label: '结果' },
  { key: 'showCalculationSteps', label: '分步计算' },
  { key: 'showExplanation', label: '公式详解' },
  { key: 'showNotes', label: '备注' },
]

const defaultName = () =>
  `${(props.schema?.resultName || props.schema?.resultSymbol || '计算书').trim() || '计算书'}.docx`

// ---- 打开：加载持久化选项 + 预览 ----
async function onOpen(): Promise<void> {
  result.value = null
  errorMsg.value = null
  progress.value = null
  try {
    options.value = await exportOptionsGet()
  } catch {
    options.value = { ...DEFAULT_EXPORT_OPTIONS }
  }
  await renderPreview()
}

// ---- 预览（选项变化时重渲，防抖） ----
let previewTimer: ReturnType<typeof setTimeout> | null = null
watch(
  options,
  () => {
    if (previewTimer) clearTimeout(previewTimer)
    previewTimer = setTimeout(() => void renderPreview(), 400)
    void persistOptions()
  },
  { deep: true },
)

async function renderPreview(): Promise<void> {
  if (!props.schema) return
  previewLoading.value = true
  try {
    const inputs = await toNumericInputs(props.schema, props.paramValues)
    previewHtml.value = await reportPreview(props.formulaId, inputs, options.value)
  } catch (e) {
    previewHtml.value = `<p style="color:#fa5151">预览生成失败：${errorMessage(e as never)}</p>`
  } finally {
    previewLoading.value = false
  }
}

async function persistOptions(): Promise<void> {
  try {
    await exportOptionsSave(options.value)
  } catch {
    /* 落盘失败不阻断导出 */
  }
}

// ---- 导出 ----
let unlisten: (() => void) | null = null

async function onExport(): Promise<void> {
  if (!props.schema || exporting.value) return
  exporting.value = true
  errorMsg.value = null
  result.value = null
  progress.value = { stage: 'prepare', current: 0, total: 0 }

  unlisten = await onTauriEvent<ExportProgress>(EXPORT_PROGRESS_EVENT, (p) => {
    progress.value = p
  })

  try {
    const inputs = await toNumericInputs(props.schema, props.paramValues)
    const r = await reportExportDocx(props.formulaId, inputs, target.value, options.value)
    result.value = r
    ElMessage.success(`已导出：${r.displayName}（${humanSize(r.sizeBytes)}）`)
  } catch (e) {
    errorMsg.value = errorMessage(e as never)
  } finally {
    if (unlisten) {
      unlisten()
      unlisten = null
    }
    exporting.value = false
  }
}

async function openExported(): Promise<void> {
  if (!result.value) return
  try {
    await openFile(result.value.path)
  } catch {
    ElMessage.warning('无法打开文件，请手动在文件夹中查找')
  }
}

async function revealExported(): Promise<void> {
  if (!result.value) return
  try {
    await reportReveal(result.value.path)
  } catch {
    ElMessage.warning('无法定位文件')
  }
}

// ---- 关闭清理 ----
function cleanup(): void {
  if (unlisten) {
    unlisten()
    unlisten = null
  }
}
</script>

<template>
  <el-dialog v-model="visible" title="导出计算书" width="min(94vw, 880px)" append-to-body>
    <div v-if="schema" class="exp">
      <!-- 章节勾选 -->
      <section class="exp__sec">
        <h4 class="exp__h">包含章节</h4>
        <div class="exp__chapters">
          <label v-for="c in CHAPTERS" :key="c.key" class="exp__chk">
            <input v-model="options[c.key]" type="checkbox" />
            <span>{{ c.label }}</span>
          </label>
        </div>
        <p v-if="isEmptySelection(options)" class="exp__empty">
          ⚠ 未勾选任何内容，将导出只有封面的空文档
        </p>
      </section>

      <!-- 免责声明 -->
      <section class="exp__sec">
        <h4 class="exp__h">免责声明</h4>
        <textarea
          v-model="options.disclaimer"
          class="exp__ta"
          rows="2"
          placeholder="留空则使用默认声明：本计算书由AI全能计算器自动生成，结果需经注册工程师复核。"
        ></textarea>
      </section>

      <!-- 保存位置 -->
      <section class="exp__sec">
        <h4 class="exp__h">保存位置</h4>
        <ExportLocationPicker v-model="target" :default-name="defaultName()" />
      </section>

      <!-- 预览 -->
      <section class="exp__sec">
        <h4 class="exp__h">预览</h4>
        <ReportPreview :html="previewHtml" :loading="previewLoading" />
      </section>

      <!-- 进度 / 结果 / 错误 -->
      <div v-if="exporting" class="exp__progress">
        <div class="exp__bar">
          <div
            class="exp__bar-fill"
            :style="{ width: progress ? progressPercent(progress) + '%' : '0%' }"
          ></div>
        </div>
        <span class="exp__bar-label">
          {{ progress ? progressStageLabel(progress.stage) : '准备中' }}
          {{ progress ? `${progress.current}/${progress.total}` : '' }}
        </span>
      </div>

      <p v-if="errorMsg" class="exp__err">⚠ {{ errorMsg }}</p>

      <div v-if="result" class="exp__done">
        <span class="exp__done-name">{{ result.displayName }}（{{ humanSize(result.sizeBytes) }}）</span>
        <button class="exp__btn" type="button" @click="openExported()">打开文件</button>
        <button class="exp__btn" type="button" @click="revealExported()">打开所在文件夹</button>
      </div>
    </div>
    <div v-else class="exp__nostate">公式未加载</div>

    <template #footer>
      <button class="exp__btn exp__btn--ghost" type="button" @click="visible = false">
        关闭
      </button>
      <button
        class="exp__btn exp__btn--primary"
        type="button"
        :disabled="!schema || exporting"
        @click="onExport()"
      >
        {{ exporting ? '导出中…' : '导出 .docx' }}
      </button>
    </template>
  </el-dialog>
</template>

<style scoped>
.exp {
  display: flex;
  flex-direction: column;
  gap: var(--sp-4);
  max-height: 70vh;
  overflow-y: auto;
}

.exp__sec {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
}

.exp__h {
  margin: 0;
  font-size: var(--f-size-sm);
  font-weight: 600;
  color: var(--c-text-2);
}

.exp__chapters {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(96px, 1fr));
  gap: var(--sp-2);
}

.exp__chk {
  display: flex;
  align-items: center;
  gap: var(--sp-1);
  font-size: var(--f-size-sm);
  color: var(--c-text);
  cursor: pointer;
}

.exp__empty {
  margin: 0;
  font-size: var(--f-size-xs);
  color: var(--c-warning);
}

.exp__ta {
  width: 100%;
  box-sizing: border-box;
  padding: var(--sp-2);
  border: var(--hairline) solid var(--c-divider);
  border-radius: var(--r-sm);
  background: var(--c-surface);
  color: var(--c-text);
  font-family: inherit;
  font-size: var(--f-size-sm);
  resize: vertical;
}

.exp__progress {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
}
.exp__bar {
  flex: 1;
  height: 6px;
  border-radius: 9999px;
  background: var(--c-surface-2);
  overflow: hidden;
}
.exp__bar-fill {
  height: 100%;
  background: var(--c-primary);
  transition: width 0.2s ease;
}
.exp__bar-label {
  font-size: var(--f-size-xs);
  color: var(--c-text-3);
  white-space: nowrap;
}

.exp__err {
  margin: 0;
  font-size: var(--f-size-sm);
  color: var(--c-danger);
}

.exp__done {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  flex-wrap: wrap;
  padding: var(--sp-2) var(--sp-3);
  border-radius: var(--r-sm);
  background: var(--c-primary-light);
}
.exp__done-name {
  font-size: var(--f-size-sm);
  color: var(--c-text);
}

.exp__nostate {
  padding: var(--sp-4);
  color: var(--c-text-3);
  text-align: center;
}

.exp__btn {
  border: var(--hairline) solid var(--c-divider);
  background: var(--c-surface);
  color: var(--c-text);
  padding: var(--sp-2) var(--sp-4);
  border-radius: var(--r-btn);
  font-family: inherit;
  font-size: var(--f-size-sm);
  cursor: pointer;
}
.exp__btn--primary {
  background: var(--c-primary);
  border-color: var(--c-primary);
  color: var(--c-text-inverse);
}
.exp__btn--primary:disabled {
  opacity: 0.5;
  cursor: default;
}
.exp__btn--ghost {
  color: var(--c-text-2);
}
[data-theme='dark'] .exp__done {
  background: var(--c-primary-light);
}
</style>
