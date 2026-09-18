<script setup lang="ts">
/**
 * ExplanationPanel —— 计算公式详解（理解需求 / 解决方式 / 分步依据）。
 *
 * 见 docs/05-项目开发方案.md §2.1.3 与 `docs/07` 的 P4-18。
 *
 * ## 三段结构来自数据契约
 *
 * `FormulaExplanation` 固定三段：`summary`（理解需求）、`solution`（解决方式）、
 * `steps[]`（分步依据，每步 `title` + `detail` + 可选 `expression`）。
 * 不按这个结构渲染就会丢内容。
 *
 * ## `{{img:N}}` 的序号从哪来
 *
 * 详解正文里的 `{{img:N}}` 的 N 对应 `schema.imageIds` 的**下标 + 1**
 * （父公式附图排在前 —— 见 `normalize_from_paste` 的 `parent_image_ids`）。
 * 所以这里用 `resolveImages(schema.imageIds)` 一次性拿全部图，
 * 再交给 `MarkdownText` 按序号内联。
 *
 * 取不到的图（已被图片管理删掉）渲染成「图片已删除」占位，
 * **不整段丢弃** —— 否则用户会以为模型没提图。
 *
 * ## 「生成详解」为什么放在这里
 *
 * 没有详解的公式（内置老数据、用户自建）在工作台里需要一个入口。
 * `explain_formula` 这条链路**不流式**（源项目亦如此），
 * 所以只有一个 loading 态，没有思考框。
 */
import { computed, ref, watch } from 'vue'
import { aiApi } from '@/api'
import MarkdownText from './MarkdownText.vue'
import MathDisplay from '@/components/math/MathDisplay.vue'
import ImageViewer from '@/components/image/ImageViewer.vue'
import { errorMessage } from '@/types/error'
import { imageAt, type ResolvedImages } from '@/types/ai'
import type { FormulaExplanation, FormulaSchema } from '@/types/domain'

const props = withDefaults(
  defineProps<{
    schema: FormulaSchema
    /** 详解（为 `null` 时显示「生成详解」引导） */
    explanation?: FormulaExplanation | null
    /** 生成中（由父组件控制时传；不传则本组件自管） */
    generating?: boolean
  }>(),
  { explanation: null, generating: undefined },
)

const emit = defineEmits<{
  /** 生成完成，把新的详解交回父组件（父组件负责落库） */
  (e: 'generated', explanation: FormulaExplanation): void
}>()

const collapsed = ref(false)

/** 分步依据（`explanation` 为 `null` 时是空数组） */
const steps = computed(() => props.explanation?.steps ?? [])

// ---------------------------------------------------------------- 图片

const images = ref<ResolvedImages>({})
const viewerOpen = ref(false)
const viewerIndex = ref(0)

/** 详解正文里真正被引用到的序号（避免为没引用的图白跑 IPC） */
const usedIndexes = computed(() => {
  const e = props.explanation
  if (!e) return [] as number[]
  const text = [e.summary, e.solution, ...e.steps.map((s) => `${s.title} ${s.detail}`)].join('\n')
  const out = new Set<number>()
  const re = /\{\{img:(\d+)\}\}/g
  let m: RegExpExecArray | null
  while ((m = re.exec(text)) !== null) {
    const n = Number(m[1])
    if (Number.isFinite(n) && n > 0) out.add(n)
  }
  return [...out].sort((a, b) => a - b)
})

/**
 * 只解析**被引用到**的图。
 *
 * `resolve_images` 的序号是按入参顺序**连续编号**的（找不到的跳过但不占位），
 * 所以不能只传子集 —— 必须传完整的 `imageIds` 才能让序号对得上。
 * 这里通过「传完整列表」保证序号正确，再按需取用。
 */
async function loadImages(): Promise<void> {
  const ids = props.schema.imageIds
  if (ids.length === 0) {
    images.value = {}
    return
  }
  try {
    images.value = await aiApi.resolveImages(ids)
  } catch {
    // 图片读不到不该让整块详解消失
    images.value = {}
  }
}

watch(() => props.schema.imageIds.join(','), loadImages, { immediate: true })

/** 查看器里的图序（按序号 1..N，缺的跳过） */
const viewerImages = computed(() =>
  usedIndexes.value.map((i) => imageAt(images.value, i)).filter((u): u is string => !!u),
)

const viewerTitles = computed(() => usedIndexes.value.map((i) => `附图 ${i}`))

function openImage(index: number): void {
  const pos = usedIndexes.value.indexOf(index)
  if (pos < 0) return
  viewerIndex.value = pos
  viewerOpen.value = true
}

// ---------------------------------------------------------------- 生成

const innerGenerating = ref(false)
const isGenerating = computed(() => props.generating ?? innerGenerating.value)

async function generate(): Promise<void> {
  if (isGenerating.value) return
  innerGenerating.value = true
  try {
    const updated = await aiApi.explainFormula(props.schema)
    if (updated.explanation) {
      emit('generated', updated.explanation)
      collapsed.value = false
      ElMessage.success('已生成详解')
    } else {
      ElMessage.warning('模型没有返回详解内容')
    }
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  } finally {
    innerGenerating.value = false
  }
}

const hasContent = computed(() => {
  const e = props.explanation
  if (!e) return false
  return Boolean(e.summary?.trim() || e.solution?.trim() || e.steps.length > 0)
})
</script>

<template>
  <section class="exp">
    <button class="exp__head" type="button" :aria-expanded="!collapsed" @click="collapsed = !collapsed">
      <span class="exp__caret" aria-hidden="true">{{ collapsed ? '▸' : '▾' }}</span>
      <span class="exp__title">计算公式详解</span>
      <span v-if="hasContent" class="exp__count">{{ steps.length }} 步</span>
    </button>

    <div v-show="!collapsed" class="exp__body">
      <!-- 没有详解：给生成入口 -->
      <div v-if="!hasContent" class="exp__empty">
        <p class="exp__empty-text">
          这条公式还没有详解。生成后会说明「为什么这么算」以及每一步的依据。
        </p>
        <el-button type="primary" :loading="isGenerating" @click="generate()">生成详解</el-button>
      </div>

      <template v-else>
        <div v-if="explanation?.summary?.trim()" class="exp__block">
          <h4 class="exp__label">理解需求</h4>
          <MarkdownText :text="explanation.summary" :images="images" @open-image="openImage" />
        </div>

        <div v-if="explanation?.solution?.trim()" class="exp__block">
          <h4 class="exp__label">解决方式</h4>
          <MarkdownText :text="explanation.solution" :images="images" @open-image="openImage" />
        </div>

        <div v-if="steps.length > 0" class="exp__block">
          <h4 class="exp__label">分步依据</h4>
          <ol class="exp__steps">
            <li v-for="(s, i) in steps" :key="i" class="exp__step">
              <span class="exp__idx">{{ i + 1 }}</span>
              <div class="exp__step-main">
                <div class="exp__step-title">
                  <MarkdownText :text="s.title" :images="images" inline @open-image="openImage" />
                </div>
                <MathDisplay
                  v-if="s.expression?.trim()"
                  class="exp__step-expr"
                  :expression="s.expression"
                  :constants="schema.constants"
                  size="sm"
                />
                <MarkdownText
                  v-if="s.detail?.trim()"
                  class="exp__step-detail"
                  :text="s.detail"
                  :images="images"
                  @open-image="openImage"
                />
              </div>
            </li>
          </ol>
        </div>

        <div class="exp__tools">
          <button class="link" type="button" :disabled="isGenerating" @click="generate()">
            {{ isGenerating ? '生成中…' : '重新生成详解' }}
          </button>
        </div>
      </template>
    </div>

    <ImageViewer
      v-model="viewerOpen"
      :images="viewerImages"
      :start-index="viewerIndex"
      :titles="viewerTitles"
    />
  </section>
</template>

<style scoped>
.exp {
  border-radius: var(--r-card);
  background: var(--c-surface);
}

.exp__head {
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

.exp__caret {
  width: 12px;
  flex-shrink: 0;
  color: var(--c-text-3);
}

.exp__title {
  font-weight: 600;
}

.exp__count {
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
}

.exp__body {
  padding: 0 var(--sp-4) var(--sp-4);
}

.exp__empty {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: var(--sp-3);
  padding: var(--sp-2) 0;
}

.exp__empty-text {
  margin: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
}

.exp__block + .exp__block {
  margin-top: var(--sp-4);
}

.exp__label {
  margin: 0 0 var(--sp-2);
  font-size: var(--f-size-sm);
  font-weight: 600;
  color: var(--c-text-2);
}

.exp__steps {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
}

.exp__step {
  display: flex;
  gap: var(--sp-3);
  padding-bottom: var(--sp-3);
  border-bottom: var(--hairline) solid var(--c-divider);
}

.exp__step:last-child {
  border-bottom: none;
  padding-bottom: 0;
}

.exp__idx {
  flex-shrink: 0;
  width: 22px;
  height: 22px;
  border-radius: 50%;
  background: var(--c-primary-light);
  color: var(--c-primary);
  font-size: var(--f-size-xs);
  line-height: 22px;
  text-align: center;
}

.exp__step-main {
  flex: 1;
  min-width: 0;
}

.exp__step-title {
  font-weight: 500;
}

.exp__step-expr {
  margin: var(--sp-2) 0;
}

.exp__step-detail {
  margin-top: var(--sp-1);
}

.exp__tools {
  margin-top: var(--sp-3);
  display: flex;
  gap: var(--sp-3);
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

.link:hover:not(:disabled) {
  text-decoration: underline;
}

.link:disabled {
  color: var(--c-text-3);
  cursor: default;
}
</style>
