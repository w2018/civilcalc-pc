<script setup lang="ts">
/**
 * 公式工作台头部：名称 / 来源 / 领域 / 收藏 / 版本入口 / 表达式。
 *
 * ## 表达式默认**展开**（需求 5）
 *
 * 之前默认折叠（一行省略）。改默认展开的理由：表达式是这条公式的
 * **身份** —— 用户打开工作台第一眼要看的就是「它到底算的是什么」。
 * 折叠成一行省略号时，长公式看不出内容，还得再点一次。
 *
 * 头部因此会高一些，但参数区本来就在下面，不影响主线操作；
 * 嫌占地方的用户点一下就能收起，选择权交回去。
 *
 * ## 收藏星标就地切换
 *
 * 收藏是最常用的动作之一，放在头部而不是藏进菜单。
 * `toggleFavorite` 返回**切换后**的状态 —— 不要用 `!之前的值` 猜
 * （并发点击时会错）。
 */
import { computed, nextTick, ref } from 'vue'
import SourceBadge from '@/components/common/SourceBadge.vue'
import MathDisplay from '@/components/math/MathDisplay.vue'
import type { FormulaSchema } from '@/types/domain'

const props = defineProps<{
  schema: FormulaSchema
  /** 是否已收藏（由父组件持有，避免这里再查一次） */
  favorite: boolean
}>()

const emit = defineEmits<{
  (e: 'toggle-favorite'): void
  (e: 'open-versions'): void
  /** 用户改了公式名（已 trim 过；父组件负责落库） */
  (e: 'rename', name: string): void
}>()

// ---------------------------------------------------------------- 改公式名

/** 是否处于内联编辑态 */
const renaming = ref(false)
/** 编辑中的名称草稿 */
const nameDraft = ref('')
/** 输入框实例（进入编辑态后聚焦） */
const nameInputRef = ref<{ focus: () => void } | null>(null)

function startRename(): void {
  nameDraft.value = props.schema.resultName ?? ''
  renaming.value = true
  // 等输入框渲染出来再聚焦
  void nextTick(() => nameInputRef.value?.focus())
}

/**
 * 提交改名。
 *
 * 空名 / 与原名相同 → 当取消处理（不写库，也不提示）——
 * 避免一次无意义的 upsert 连带重建检索索引。
 *
 * ⚠️ `renaming` 先置 false 再 emit：`el-input` 卸载时可能补一次 blur，
 * 那次会再调到这里，靠这个早退挡住重复提交。
 */
function commitRename(): void {
  if (!renaming.value) return
  renaming.value = false
  const next = nameDraft.value.trim()
  if (!next || next === (props.schema.resultName ?? '').trim()) return
  emit('rename', next)
}

/** `Esc` 放弃修改 */
function cancelRename(): void {
  renaming.value = false
}

/** 表达式是否展开（需求 5：默认展开） */
const expanded = ref(true)

const sourceKind = computed(() => props.schema.source.kind)
const verified = computed(() => props.schema.source.verified)

/** 领域 + 标签，去重后拼接 */
const chips = computed(() => {
  const out: string[] = []
  const d = props.schema.domain?.trim()
  if (d) out.push(d)
  for (const t of props.schema.tags ?? []) {
    const s = t?.trim()
    if (s && !out.includes(s)) out.push(s)
  }
  return out
})

/** 规范引用（如 `GB 50010-2010 第6.2.10条`） */
const refText = computed(() => props.schema.source.ref?.trim() ?? '')

/** 是否有表达式可展示 */
const hasExpression = computed(() => (props.schema.expression ?? '').trim().length > 0)
</script>

<template>
  <header class="head">
    <div class="head__top">
      <!-- 改名：点 🖊 变输入框，Enter/失焦提交，Esc 放弃 -->
      <el-input
        v-if="renaming"
        ref="nameInputRef"
        v-model="nameDraft"
        class="head__name-input"
        size="small"
        maxlength="60"
        placeholder="公式名"
        @keyup.enter="commitRename"
        @keyup.esc="cancelRename"
        @blur="commitRename"
      />
      <h1 v-else class="head__name">{{ schema.resultName || '未命名公式' }}</h1>

      <el-tooltip v-if="!renaming" content="修改公式名" placement="bottom">
        <button
          class="head__icon-btn"
          type="button"
          aria-label="修改公式名"
          @click="startRename"
        >
          🖊
        </button>
      </el-tooltip>

      <el-tooltip :content="favorite ? '取消收藏' : '收藏'" placement="bottom">
        <button
          class="head__icon-btn"
          type="button"
          :class="{ 'head__star--on': favorite }"
          :aria-label="favorite ? '取消收藏' : '收藏'"
          :aria-pressed="favorite"
          @click="emit('toggle-favorite')"
        >
          {{ favorite ? '★' : '☆' }}
        </button>
      </el-tooltip>

      <button class="head__link" type="button" @click="emit('open-versions')">版本</button>
    </div>

    <div class="head__meta">
      <SourceBadge :kind="sourceKind" :verified="verified" />
      <span v-if="schema.resultSymbol" class="head__symbol">
        {{ schema.resultSymbol }}
        <span v-if="schema.resultUnit" class="head__unit">{{ schema.resultUnit }}</span>
      </span>
      <span v-for="c in chips" :key="c" class="head__chip">{{ c }}</span>
    </div>

    <div v-if="refText" class="head__ref">
      <span class="head__ref-key">依据</span>
      <span class="head__ref-val">{{ refText }}</span>
    </div>

    <div v-if="hasExpression" class="head__expr">
      <button
        class="head__expr-toggle"
        type="button"
        :aria-expanded="expanded"
        @click="expanded = !expanded"
      >
        <span class="head__caret" aria-hidden="true">{{ expanded ? '▾' : '▸' }}</span>
        <span>表达式</span>
      </button>

      <!-- 展开：二维排版（双击或点开全屏看完整公式） -->
      <MathDisplay
        v-if="expanded"
        class="head__expr-math"
        :expression="schema.expression"
        :constants="schema.constants"
        :size="expanded ? 'base' : 'sm'"
      />
      <!-- 折叠：单行等宽原文（保持头部紧凑） -->
      <code v-else class="head__expr-code head__expr-code--clamp">
        {{ schema.expression }}
      </code>
    </div>
  </header>
</template>

<style scoped>
.head {
  padding: var(--sp-4);
  border-radius: var(--r-card);
  background: var(--c-surface);
}

.head__top {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
}

.head__name {
  flex: 1;
  min-width: 0;
  margin: 0;
  font-size: var(--f-size-xl);
  font-weight: 600;
  color: var(--c-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 名称右侧的一排图标按钮（改名 🖊 / 收藏 ★）共用同一套外观 */
.head__icon-btn {
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

.head__icon-btn:hover {
  background: var(--c-surface-hover);
}

/* 收藏选中态（只改颜色，外观仍走 .head__icon-btn） */
.head__star--on {
  color: var(--c-warning);
}

/* 改名时的内联输入框：宽度跟着名称走，别把整行撑开 */
.head__name-input {
  flex: 0 1 auto;
  width: 320px;
  max-width: 60%;
}

.head__link {
  flex-shrink: 0;
  border: none;
  background: transparent;
  padding: 0 var(--sp-1);
  color: var(--c-primary);
  font-family: inherit;
  font-size: var(--f-size-sm);
  cursor: pointer;
}

.head__link:hover {
  text-decoration: underline;
}

.head__meta {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--sp-2);
  margin-top: var(--sp-2);
}

.head__symbol {
  font-family: var(--f-mono);
  font-size: var(--f-size-sm);
  color: var(--c-text-2);
}

.head__unit {
  color: var(--c-text-3);
}

.head__chip {
  padding: 1px var(--sp-2);
  border-radius: var(--r-pill);
  background: var(--c-surface-2);
  color: var(--c-text-2);
  font-size: var(--f-size-xs);
}

.head__ref {
  display: flex;
  gap: var(--sp-2);
  margin-top: var(--sp-2);
  font-size: var(--f-size-sm);
}

.head__ref-key {
  flex-shrink: 0;
  color: var(--c-text-3);
}

.head__ref-val {
  min-width: 0;
  color: var(--c-text-2);
}

.head__expr {
  margin-top: var(--sp-3);
  padding-top: var(--sp-3);
  border-top: var(--hairline) solid var(--c-divider);
}

.head__expr-toggle {
  display: flex;
  align-items: center;
  gap: var(--sp-1);
  border: none;
  background: transparent;
  padding: 0;
  color: var(--c-text-3);
  font-family: inherit;
  font-size: var(--f-size-sm);
  cursor: pointer;
}

.head__caret {
  width: 12px;
}

.head__expr-code {
  display: block;
  margin-top: var(--sp-2);
  font-family: var(--f-mono);
  font-size: var(--f-size-sm);
  line-height: 1.7;
  color: var(--c-text);
  word-break: break-all;
  white-space: pre-wrap;
}

/* 折叠时只留一行 */
.head__expr-code--clamp {
  display: -webkit-box;
  -webkit-line-clamp: 1;
  line-clamp: 1;
  -webkit-box-orient: vertical;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
</style>
