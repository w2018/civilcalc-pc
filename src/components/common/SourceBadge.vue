<script setup lang="ts">
/**
 * 来源徽章（`STANDARD` / `AI` / `CUSTOM` / `DERIVED`）。
 *
 * 颜色走 `--c-src-*` 令牌（见 `tokens.css`）：
 *
 * | 来源 | 浅色主题 | 暗色主题 | 含义 |
 * |---|---|---|---|
 * | `STANDARD` | 🔵 蓝灰 | 灰 | 来自规范条文 |
 * | `AI` | 🟣 紫 | 浅紫 | AI 解析生成 |
 * | `CUSTOM` | ⚪ 灰 | 灰 | 用户自建 |
 * | `DERIVED` | 🟢 绿 | 绿 | 由其他公式派生 |
 *
 * `verified` 为真时加一个 ✓ 后缀 —— 「AI 生成但已人工核对」与
 * 「AI 生成未核对」在信任度上差别很大，值得在列表里直接区分。
 */
import { computed } from 'vue'
import { sourceKindLabel, type SourceKind } from '@/types/domain'

const props = withDefaults(
  defineProps<{
    kind: SourceKind
    /** 是否已人工核对（仅对 `AI` 来源有意义，但任何来源都可标） */
    verified?: boolean
    /** 紧凑模式：只显示颜色点 + 单字，用于表格内 */
    compact?: boolean
  }>(),
  { verified: false, compact: false },
)

const label = computed(() => sourceKindLabel(props.kind))

/** 与 `--c-src-*` 一一对应 */
const colorVar = computed(() => `var(--c-src-${props.kind.toLowerCase()})`)

/** 紧凑模式取首字（规范 / AI / 自建 / 派生） */
const shortLabel = computed(() => {
  switch (props.kind) {
    case 'STANDARD':
      return '规'
    case 'AI':
      return 'AI'
    case 'CUSTOM':
      return '自'
    case 'DERIVED':
      return '派'
  }
})
</script>

<template>
  <span
    class="badge"
    :class="{ 'badge--compact': compact }"
    :style="{ '--badge-color': colorVar }"
    :title="`${label}${verified ? '（已核对）' : ''}`"
  >
    <span class="badge__dot" aria-hidden="true" />
    <span class="badge__text">{{ compact ? shortLabel : label }}</span>
    <span v-if="verified" class="badge__verified" aria-label="已人工核对">✓</span>
  </span>
</template>

<style scoped>
.badge {
  display: inline-flex;
  align-items: center;
  gap: var(--sp-1);
  flex-shrink: 0;
  padding: 1px var(--sp-2);
  border: var(--hairline) solid var(--badge-color);
  border-radius: var(--r-pill);
  color: var(--badge-color);
  font-size: var(--f-size-xs);
  line-height: 1.6;
  white-space: nowrap;
}

.badge--compact {
  padding: 1px var(--sp-1);
}

.badge__dot {
  width: 6px;
  height: 6px;
  border-radius: var(--r-pill);
  background: var(--badge-color);
}

.badge--compact .badge__dot {
  display: none;
}

.badge__verified {
  font-weight: 700;
}
</style>
