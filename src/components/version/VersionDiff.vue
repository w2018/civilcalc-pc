<script setup lang="ts">
/**
 * 两版本 6 维差异展示（P3-16）。
 *
 * 见 docs/08-IPC契约.md 组 6 与 `types/domain.ts` 的 `VersionDiff`。
 *
 * 差异维度固定顺序（后端已排好）：
 * `expression` → `altExpressions` → `variables` → `source.ref` →
 * `source.verified` → `constants`。
 *
 * 每项：`oldValue`（来自 A）→ `newValue`（来自 B）。空串表示「无」。
 */
import type { VersionDiff } from '@/types/domain'

defineProps<{
  diff: VersionDiff | null
  /** 版本 A 标签（如 `v2`） */
  labelA?: string
  /** 版本 B 标签（如 `v3`） */
  labelB?: string
}>()

function isEmpty(v: string): boolean {
  return !v || v === '—'
}
</script>

<template>
  <div class="vd">
    <p v-if="!diff || diff.changes.length === 0" class="vd__none">两版无差异</p>

    <table v-else class="vd__table">
      <thead>
        <tr>
          <th>维度</th>
          <th>{{ labelA || '旧版' }}</th>
          <th>{{ labelB || '新版' }}</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="c in diff.changes" :key="c.field" class="vd__row">
          <td class="vd__field">{{ c.label }}</td>
          <td class="vd__old">
            <code>{{ isEmpty(c.oldValue) ? '（无）' : c.oldValue }}</code>
          </td>
          <td class="vd__new">
            <code>{{ isEmpty(c.newValue) ? '（无）' : c.newValue }}</code>
          </td>
        </tr>
      </tbody>
    </table>
  </div>
</template>

<style scoped>
.vd {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
}

.vd__none {
  margin: 0;
  font-size: var(--f-size-sm);
  color: var(--c-text-3);
}

.vd__table {
  width: 100%;
  border-collapse: collapse;
  font-size: var(--f-size-sm);
}
.vd__table th {
  text-align: left;
  color: var(--c-text-3);
  font-weight: 500;
  padding: var(--sp-1) var(--sp-2);
  border-bottom: var(--hairline) solid var(--c-divider);
}
.vd__row td {
  padding: var(--sp-2);
  border-bottom: var(--hairline) solid var(--c-divider);
  vertical-align: top;
}
.vd__field {
  color: var(--c-text-2);
  white-space: nowrap;
}
.vd__old code,
.vd__new code {
  font-family: var(--f-mono);
  font-size: var(--f-size-xs);
  white-space: pre-wrap;
  word-break: break-all;
}
.vd__old code {
  color: var(--c-text-3);
  text-decoration: line-through;
}
.vd__new code {
  color: var(--c-text);
}
</style>
