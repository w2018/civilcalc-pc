<script setup lang="ts">
/**
 * Excel 公式面板（P3-13）。
 *
 * 见 docs/08-IPC契约.md 组 12a、docs/06-Excel单元格映射契约.md 与 `types/excel.ts`。
 *
 * ## 两模式切换
 *
 * - `ref` 引用式（`=PI()*A1^2*B1`）：贴进 Excel 复用，变量留在第 1 行，改数即重算
 * - `value` 带数值式（`=PI()*2^2*3`）：写进计算书，数值已代入
 *
 * `ref` 模式忽略输入值（公式里本来就没有数）。
 *
 * ## 🔴 三个诊断字段必须显著展示
 *
 * 后端 `convert` 有两处静默行为，会让用户拿到一份看不出来的问题公式：
 *
 * | 字段 | 含义 | 样式 |
 * |---|---|---|
 * | `unresolvedNames` | 公式里未声明的标识符（Excel 里 `#NAME?`） | 🔴 红条 |
 * | `unresolvedRefs` | 值模式数值非法（NaN）残留的引用 | 🔴 红条 |
 * | `missingInputs` | 值模式漏填的参数（按 0 代入、**无告警**） | 🟡 黄条 |
 *
 * `missingInputs` 是工程计算里最容易出事的错误，所以黄条也要显眼。
 *
 * ## 逐条复制
 *
 * 主公式 / 每条多结果 / 每个分支 / 整张参数表，都能单独复制。
 */
import { computed, ref, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { excelConvert } from '@/api/excel'
import { copyText } from '@/utils/clipboard'
import { toNumericInputs } from '@/utils/inputs'
import type { FormulaSchema } from '@/types/domain'
import type { ExcelMode, ExcelResult } from '@/types/excel'

const props = defineProps<{
  schema: FormulaSchema
  /** 用户原始输入（字符串，可为算式）；ExcelPanel 自行解析成数值 */
  paramValues: Record<string, string>
}>()

const emit = defineEmits<{
  (e: 'copied', text: string): void
}>()

const mode = ref<ExcelMode>('ref')
const result = ref<ExcelResult | null>(null)
const loading = ref(false)
const error = ref(false)

/** 把原始输入解析成数值（算式走 `evalExpression`；空/失败 → 跳过） */
async function buildInputs(): Promise<Record<string, number>> {
  return toNumericInputs(props.schema, props.paramValues)
}

async function convert(): Promise<void> {
  loading.value = true
  error.value = false
  try {
    const inputs = mode.value === 'value' ? await buildInputs() : {}
    result.value = await excelConvert(props.schema, mode.value, inputs)
  } catch (e) {
    error.value = true
    result.value = null
    console.warn('[ExcelPanel] 转换失败', e)
  } finally {
    loading.value = false
  }
}

watch(
  () => [props.schema, props.paramValues, mode.value],
  () => void convert(),
  { immediate: true },
)

const hasDiag = computed(
  () =>
    !!result.value &&
    (result.value.unresolvedNames.length > 0 ||
      result.value.unresolvedRefs.length > 0 ||
      result.value.missingInputs.length > 0),
)

/** 复制一段文本并提示 */
async function copy(text: string, label: string): Promise<void> {
  if (!text) return
  if (await copyText(text)) {
    ElMessage.success(`已复制${label}`)
    emit('copied', text)
  } else {
    ElMessage.warning('复制失败，请手动选中')
  }
}

/** 复制整张参数对照表（便于直接填进 Excel 第 1 行） */
function copyParamTable(): void {
  if (!result.value) return
  const lines = result.value.paramMapping.map(
    (p) => `${p.cellRef}\t${p.variable}\t${p.desc}${p.unit ? ` (${p.unit})` : ''}`,
  )
  void copy(lines.join('\n'), '参数表')
}

/** 复制全部公式（主 + 多结果 + 分支） */
function copyAll(): void {
  if (!result.value) return
  const r = result.value
  const lines = [r.mainFormula]
  for (const o of r.outputs) lines.push(o.formula)
  for (const b of r.branches) lines.push(b.formula)
  void copy(lines.join('\n'), '全部公式')
}

function onModeChange(m: ExcelMode): void {
  mode.value = m
}
</script>

<template>
  <div class="excel">
    <!-- 工具条：模式切换 + 复制全部 -->
    <div class="excel__bar">
      <div class="excel__seg" role="tablist" aria-label="Excel 公式模式">
        <button
          class="excel__seg-btn"
          type="button"
          role="tab"
          :aria-selected="mode === 'ref'"
          :class="{ 'excel__seg-btn--on': mode === 'ref' }"
          @click="onModeChange('ref')"
        >
          引用式
        </button>
        <button
          class="excel__seg-btn"
          type="button"
          role="tab"
          :aria-selected="mode === 'value'"
          :class="{ 'excel__seg-btn--on': mode === 'value' }"
          @click="onModeChange('value')"
        >
          带数值式
        </button>
      </div>
      <button class="excel__link" type="button" :disabled="!result" @click="copyAll()">
        复制全部
      </button>
    </div>

    <p class="excel__hint">
      {{ mode === 'ref' ? '变量留在第 1 行，贴进 Excel 改数即重算' : '数值已代入，适合写进计算书' }}
    </p>

    <!-- 加载 / 错误 -->
    <div v-if="loading" class="excel__state">转换中…</div>
    <div v-else-if="error" class="excel__state excel__state--err">Excel 公式转换失败</div>

    <template v-else-if="result">
      <!-- 🔴 诊断条 -->
      <div v-if="hasDiag" class="excel__diag">
        <div v-if="result.unresolvedNames.length" class="excel__diag-row excel__diag-row--err">
          ⚠ 公式含未声明符号：{{ result.unresolvedNames.join('、') }}（Excel 中将显示 #NAME?）
        </div>
        <div v-if="result.unresolvedRefs.length" class="excel__diag-row excel__diag-row--err">
          ⚠ 数值非法残留引用：{{ result.unresolvedRefs.join('、') }}
        </div>
        <div v-if="result.missingInputs.length" class="excel__diag-row excel__diag-row--warn">
          ⚠ 以下参数未填值，已按 0 代入：{{ result.missingInputs.join('、') }}
        </div>
      </div>

      <!-- 主公式 -->
      <div class="excel__row">
        <code class="excel__formula">{{ result.mainFormula }}</code>
        <button class="excel__copy" type="button" @click="copy(result.mainFormula, '主公式')">
          复制
        </button>
      </div>

      <!-- 多结果 -->
      <div v-for="o in result.outputs" :key="o.symbol" class="excel__row">
        <span class="excel__tag">{{ o.name || o.symbol }}</span>
        <code class="excel__formula">{{ o.formula }}</code>
        <button class="excel__copy" type="button" @click="copy(o.formula, o.name || o.symbol)">
          复制
        </button>
      </div>

      <!-- 分支 -->
      <div v-for="(b, i) in result.branches" :key="`b${i}`" class="excel__row">
        <span class="excel__tag">{{ b.label }}</span>
        <code class="excel__formula">{{ b.formula }}</code>
        <button class="excel__copy" type="button" @click="copy(b.formula, b.label)">复制</button>
      </div>

      <!-- 参数对照表 -->
      <div class="excel__map">
        <div class="excel__map-head">
          <span class="excel__map-title">参数对照表</span>
          <button class="excel__link" type="button" @click="copyParamTable()">复制表格</button>
        </div>
        <table class="excel__table">
          <thead>
            <tr>
              <th>单元格</th>
              <th>变量</th>
              <th>说明</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="p in result.paramMapping" :key="p.variable">
              <td class="excel__cell">{{ p.cellRef }}</td>
              <td class="excel__var">{{ p.variable }}</td>
              <td class="excel__desc">
                {{ p.desc }}<span v-if="p.unit" class="excel__unit"> ({{ p.unit }})</span>
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <!-- 用到的函数 -->
      <details v-if="result.usedFunctions.length" class="excel__fn">
        <summary>用到的 Excel 函数（{{ result.usedFunctions.length }}）</summary>
        <ul class="excel__fn-list">
          <li v-for="f in result.usedFunctions" :key="f.name">
            <code>{{ f.name }}</code> — {{ f.description }}
          </li>
        </ul>
      </details>

      <!-- 告警 -->
      <p v-if="result.warnings.length" class="excel__warn">
        {{ result.warnings.join('；') }}
      </p>
    </template>
  </div>
</template>

<style scoped>
.excel {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
}

.excel__bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--sp-2);
}

/* 分段控件：微信风用底线 + 主色高亮 */
.excel__seg {
  display: inline-flex;
  border-bottom: var(--hairline) solid var(--c-divider);
}
.excel__seg-btn {
  border: none;
  background: transparent;
  padding: var(--sp-1) var(--sp-3);
  color: var(--c-text-3);
  font-family: inherit;
  font-size: var(--f-size-sm);
  cursor: pointer;
}
.excel__seg-btn--on {
  color: var(--c-primary);
  border-bottom: 2px solid var(--c-primary);
  margin-bottom: -1px;
}

.excel__link {
  border: none;
  background: transparent;
  padding: 0;
  color: var(--c-primary);
  font-family: inherit;
  font-size: var(--f-size-sm);
  cursor: pointer;
}
.excel__link:disabled {
  color: var(--c-text-3);
  cursor: default;
}
.excel__link:hover:not(:disabled) {
  text-decoration: underline;
}

.excel__hint {
  margin: 0;
  font-size: var(--f-size-xs);
  color: var(--c-text-3);
}

.excel__state {
  padding: var(--sp-3);
  font-size: var(--f-size-sm);
  color: var(--c-text-3);
}
.excel__state--err {
  color: var(--c-danger);
}

/* 诊断条 */
.excel__diag {
  display: flex;
  flex-direction: column;
  gap: 2px;
  border-radius: var(--r-sm);
  overflow: hidden;
}
.excel__diag-row {
  padding: var(--sp-1) var(--sp-2);
  font-size: var(--f-size-xs);
  line-height: 1.5;
}
.excel__diag-row--err {
  background: var(--c-surface-2);
  color: var(--c-danger);
  border-left: 3px solid var(--c-danger);
}
.excel__diag-row--warn {
  background: #fff7e6;
  color: #ad6800;
  border-left: 3px solid var(--c-warning);
}
[data-theme='dark'] .excel__diag-row--warn {
  background: #3a2e12;
  color: #e0a64b;
}

/* 公式行 */
.excel__row {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  padding: var(--sp-1) 0;
}
.excel__tag {
  flex-shrink: 0;
  min-width: 48px;
  font-size: var(--f-size-xs);
  color: var(--c-text-3);
}
.excel__formula {
  flex: 1;
  min-width: 0;
  font-family: var(--f-mono);
  font-size: var(--f-size-sm);
  color: var(--c-text);
  word-break: break-all;
}
.excel__copy {
  flex-shrink: 0;
  border: none;
  background: transparent;
  padding: 0;
  color: var(--c-primary);
  font-family: inherit;
  font-size: var(--f-size-xs);
  cursor: pointer;
}
.excel__copy:hover {
  text-decoration: underline;
}

/* 参数表 */
.excel__map {
  margin-top: var(--sp-2);
  border-top: var(--hairline) solid var(--c-divider);
  padding-top: var(--sp-2);
}
.excel__map-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: var(--sp-1);
}
.excel__map-title {
  font-size: var(--f-size-sm);
  color: var(--c-text-2);
}
.excel__table {
  width: 100%;
  border-collapse: collapse;
  font-size: var(--f-size-xs);
}
.excel__table th {
  text-align: left;
  color: var(--c-text-3);
  font-weight: 500;
  padding: 4px var(--sp-2);
  border-bottom: var(--hairline) solid var(--c-divider);
}
.excel__table td {
  padding: 4px var(--sp-2);
  border-bottom: var(--hairline) solid var(--c-divider);
  color: var(--c-text-2);
}
.excel__cell {
  font-family: var(--f-mono);
  color: var(--c-text);
}
.excel__var {
  font-family: var(--f-mono);
  color: var(--c-math-symbol);
}
.excel__unit {
  color: var(--c-text-3);
}

/* 函数说明 */
.excel__fn {
  margin-top: var(--sp-2);
  font-size: var(--f-size-xs);
}
.excel__fn summary {
  cursor: pointer;
  color: var(--c-text-3);
}
.excel__fn-list {
  margin: var(--sp-1) 0 0;
  padding-left: var(--sp-4);
  color: var(--c-text-2);
  line-height: 1.7;
}
.excel__fn code {
  font-family: var(--f-mono);
  color: var(--c-math-function);
}

.excel__warn {
  margin: var(--sp-1) 0 0;
  font-size: var(--f-size-xs);
  color: var(--c-text-3);
  line-height: 1.5;
}
</style>
