<script setup lang="ts">
/**
 * RemoteFileList —— 云端备份列表（最多 20 条 + 总数提示 + 上次备份统计）。
 *
 * 见 docs/05-项目开发方案.md「流程 5」与 `types/backup.ts` 的 `RemoteListResult`。
 *
 * ## 🔴 排序与展示都用 `timestampMs`，不用 `lastModifiedMs`
 *
 * `timestampMs` 来自**文件名里的时间戳**（本机生成时写进去的），
 * `lastModifiedMs` 是服务端给的 —— 有的网盘不返回，或返回 UTC 却标成本地。
 * 用后者会出现「备份时间差 8 小时」这种迷惑现象。
 *
 * ## 🔴 只列 20 条，但要说清「还有更多」
 *
 * 后端 `LIST_LIMIT = 20`，`total` 是服务器上的总数。
 * 不提示的话，用户会以为「我备份了 30 次怎么只剩 20 条」。
 *
 * ## 删除是**破坏性且不可撤销**的
 *
 * 本组件只发出意图（`delete` 事件），二次确认由父组件做 ——
 * 因为确认框还要展示「这条是什么时候的备份」，那是父组件更清楚的信息。
 */
import { computed } from 'vue'
import { formatDateTime } from '@/utils/format'
import { humanSize, type RemoteBackup } from '@/types/backup'

const props = withDefaults(
  defineProps<{
    backups: RemoteBackup[]
    /** 服务器上的**总数**（截断前） */
    total: number
    loading?: boolean
    /** 有上传/下载在进行（禁用所有操作） */
    busy?: boolean
    /** 上次成功备份时间（毫秒）；`null` 表示还没备份过 */
    lastBackupAt?: number | null
    /** 累计上传次数 */
    backupCount?: number
  }>(),
  { loading: false, busy: false, lastBackupAt: null, backupCount: 0 },
)

const emit = defineEmits<{
  (e: 'refresh'): void
  (e: 'import', backup: RemoteBackup): void
  (e: 'delete', backup: RemoteBackup): void
}>()

// 时间显示统一走 `utils/format` 的 `formatDateTime`（完整年月日时分秒）。
// 这里原本自己抄了一份、只到分，与其他页面精度不一致，已删。

const hiddenCount = computed(() => Math.max(0, props.total - props.backups.length))

const statsText = computed(() => {
  const at = props.lastBackupAt
  const last = at ? formatDateTime(at) : '还没备份过'
  return `上次备份：${last} · 累计 ${props.backupCount} 次`
})
</script>

<template>
  <div class="rfl">
    <!-- 统计 -->
    <div class="rfl__stats">
      <span class="rfl__stat">{{ statsText }}</span>
      <el-button size="small" :disabled="busy || loading" @click="emit('refresh')">刷新列表</el-button>
    </div>

    <el-skeleton v-if="loading" :rows="3" animated />

    <div v-else-if="backups.length === 0" class="rfl__empty">
      <p class="rfl__empty-title">云端还没有备份</p>
      <p class="rfl__empty-text">点上方「上传备份」把当前数据打包传上去，之后换机就能一键恢复。</p>
    </div>

    <template v-else>
      <ul class="rfl__list">
        <li v-for="b in backups" :key="b.name" class="row">
          <div class="row__main">
            <span class="row__time">{{ formatDateTime(b.timestampMs) }}</span>
            <span class="row__meta">{{ humanSize(b.sizeBytes) }}</span>
            <span class="row__name">{{ b.name }}</span>
          </div>
          <div class="row__ops">
            <button class="link" type="button" :disabled="busy" @click="emit('import', b)">
              下载导入
            </button>
            <button class="link link--danger" type="button" :disabled="busy" @click="emit('delete', b)">
              删除
            </button>
          </div>
        </li>
      </ul>

      <p v-if="hiddenCount > 0" class="rfl__more">
        只显示最近 {{ backups.length }} 条，服务器上还有 {{ hiddenCount }} 条更早的备份。
      </p>
    </template>
  </div>
</template>

<style scoped>
.rfl {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
}

.rfl__stats {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--sp-3);
}

.rfl__stat {
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
}

.rfl__empty {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
  padding: var(--sp-5) var(--sp-4);
  text-align: center;
}

.rfl__empty-title {
  margin: 0;
  color: var(--c-text);
  font-size: var(--f-size-base);
  font-weight: 600;
}

.rfl__empty-text {
  margin: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
  line-height: 1.7;
}

.rfl__list {
  margin: 0;
  padding: 0;
  list-style: none;
}

.row {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
  padding: var(--sp-2) 0;
  border-bottom: var(--hairline) solid var(--c-divider);
}

.row:last-child {
  border-bottom: none;
}

.row__main {
  display: flex;
  align-items: baseline;
  gap: var(--sp-3);
  flex: 1;
  min-width: 0;
}

.row__time {
  flex-shrink: 0;
  color: var(--c-text);
  font-size: var(--f-size-sm);
  font-variant-numeric: tabular-nums;
}

.row__meta {
  flex-shrink: 0;
  color: var(--c-text-2);
  font-size: var(--f-size-xs);
}

.row__name {
  color: var(--c-text-3);
  font-family: var(--f-mono);
  font-size: var(--f-size-xs);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.row__ops {
  display: flex;
  gap: var(--sp-3);
  flex-shrink: 0;
}

.rfl__more {
  margin: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
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

.link--danger {
  color: var(--c-danger);
}

.link:hover:not(:disabled) {
  text-decoration: underline;
}

.link:disabled {
  color: var(--c-text-3);
  cursor: default;
}
</style>
