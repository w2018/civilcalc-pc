<script setup lang="ts">
/**
 * 公式列表行（微信风卡片行）—— 见 docs/05 §2.3.4。
 *
 * ```
 * ┌──────────────────────────────────────────────────────┐
 * │  梁正截面受弯承载力                            ★  ›  │
 * │  [规范] 结构 · As  mm²                               │
 * └──────────────────────────────────────────────────────┘
 * ```
 *
 * | 规范项 | 值 |
 * |---|---|
 * | 行高 | 56px |
 * | 名称 | 16px |
 * | 副行 | 来源徽章 + 领域 + 结果符号（13px） |
 * | hover | **仅换底**，不上浮不加阴影 |
 * | 右箭头 | 表示"可进入" |
 *
 * 搜索 / 收藏 / 公式库三处共用，所以抽成组件。
 */
import { computed } from 'vue'
import SourceBadge from '@/components/common/SourceBadge.vue'
import type { FormulaSchema, SearchSuggestion, SourceKind } from '@/types/domain'

/** 兼容完整 Schema 与轻量建议两种输入 */
type RowInput = FormulaSchema | SearchSuggestion

const props = withDefaults(
  defineProps<{
    item: RowInput
    /** 是否已收藏（仅完整 Schema 场景传） */
    favorite?: boolean
    /** 是否显示收藏星标（搜索建议里没有收藏态，隐藏） */
    showStar?: boolean
  }>(),
  { favorite: false, showStar: true },
)

const emit = defineEmits<{
  (e: 'open'): void
  (e: 'toggle-favorite'): void
}>()

/** 类型收窄：有 `variables` 字段的就是完整 Schema */
function isSchema(x: RowInput): x is FormulaSchema {
  return 'variables' in x
}

const title = computed(() => props.item.resultName || '未命名公式')

/**
 * 来源类别。
 *
 * 两种输入的字段名不同：
 * - `FormulaSchema` → `source.kind`
 * - `SearchSuggestion` → `sourceKind`（扁平，为了减小 IPC 负载）
 */
const kind = computed<SourceKind>(() =>
  isSchema(props.item) ? props.item.source.kind : props.item.sourceKind,
)

const symbol = computed(() => {
  if (!isSchema(props.item)) return ''
  return props.item.resultSymbol?.trim() ?? ''
})

const unit = computed(() => {
  if (!isSchema(props.item)) return ''
  return props.item.resultUnit?.trim() ?? ''
})

const domain = computed(() => props.item.domain?.trim() ?? '')
</script>

<template>
  <div
    class="row"
    role="button"
    tabindex="0"
    @click="emit('open')"
    @keydown.enter.prevent="emit('open')"
    @keydown.space.prevent="emit('open')"
  >
    <div class="row__main">
      <div class="row__title">{{ title }}</div>
      <div class="row__sub">
        <SourceBadge :kind="kind" compact />
        <span v-if="domain" class="row__domain">{{ domain }}</span>
        <span v-if="symbol" class="row__symbol">
          {{ symbol }}<span v-if="unit" class="row__unit"> {{ unit }}</span>
        </span>
      </div>
    </div>

    <button
      v-if="showStar"
      class="row__star"
      type="button"
      :class="{ 'row__star--on': favorite }"
      :aria-label="favorite ? '取消收藏' : '收藏'"
      :aria-pressed="favorite"
      @click.stop="emit('toggle-favorite')"
    >
      {{ favorite ? '★' : '☆' }}
    </button>

    <span class="row__arrow" aria-hidden="true">›</span>
  </div>
</template>

<style scoped>
.row {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  min-height: var(--row-h);
  padding: var(--sp-2) var(--sp-4);
  background: var(--c-surface);
  cursor: pointer;
}

/* hover 仅换底（规范：不上浮、不加阴影） */
.row:hover {
  background: var(--c-surface-hover);
}

.row__main {
  flex: 1;
  min-width: 0;
}

.row__title {
  font-size: var(--f-size-lg);
  color: var(--c-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.row__sub {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  margin-top: 2px;
  font-size: var(--f-size-sm);
  color: var(--c-text-3);
  overflow: hidden;
}

.row__domain,
.row__symbol {
  flex-shrink: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.row__symbol {
  font-family: var(--f-mono);
}

.row__unit {
  font-family: var(--f-sans);
  color: var(--c-text-3);
}

.row__star {
  flex-shrink: 0;
  width: 30px;
  height: 30px;
  border: none;
  border-radius: var(--r-btn);
  background: transparent;
  color: var(--c-text-3);
  font-size: var(--f-size-lg);
  line-height: 1;
  cursor: pointer;
}

.row__star:hover {
  background: var(--c-surface-hover);
}

.row__star--on {
  color: var(--c-warning);
}

.row__arrow {
  flex-shrink: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-xl);
  line-height: 1;
}
</style>
