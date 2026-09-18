<script setup lang="ts">
/**
 * ProgressPanel —— 备份上传 / 下载 / 导入的进度与取消。
 *
 * 见 docs/05-项目开发方案.md「流程 5：备份与恢复」。
 *
 * ## 🔴 `total <= 0` 时**不能**显示百分比
 *
 * 网盘不给 `Content-Length` 时 `total` 是 0（后端刻意不用 `-1`：
 * `current/total` 都是无符号数）。此时只能显示「已传输 x MB」——
 * 硬凑一个百分比会让用户看着进度条卡在 0% 直到突然完成。
 *
 * ## 取消是「立即断流」
 *
 * `webdavCancel()` 让后端在下一个网络分块处断开连接，
 * 并让进行中的 `invoke` 以 `cancelled` 拒绝。
 * 所以本组件只负责**发出取消请求**，收尾（关面板、静默不报错）
 * 由父组件在 `catch` 里用 `isCancelled()` 处理。
 */
import { computed } from 'vue'
import { backupPercent, backupStageLabel, humanSize, type BackupProgress } from '@/types/backup'

const props = withDefaults(
  defineProps<{
    /** 最近一次进度（为 `null` 时显示「准备中」） */
    progress: BackupProgress | null
    /** 是否正在进行 */
    busy: boolean
    /** 标题（如「上传到 WebDAV」） */
    title?: string
    /** 是否可取消 */
    cancellable?: boolean
  }>(),
  { title: '正在处理', cancellable: true },
)

const emit = defineEmits<{
  (e: 'cancel'): void
}>()

/** 百分比；`null` 表示「长度未知，只能看已传字节」 */
const percent = computed(() => {
  const p = props.progress
  if (!p || p.total <= 0) return null
  return backupPercent(p)
})

const stageText = computed(() => (props.progress ? backupStageLabel(props.progress.stage) : '准备中'))

const detail = computed(() => {
  const p = props.progress
  if (!p) return ''
  if (p.total <= 0) return `已传输 ${humanSize(p.current)}`
  return `${humanSize(p.current)} / ${humanSize(p.total)}`
})
</script>

<template>
  <section v-if="busy" class="prog">
    <div class="prog__head">
      <span class="prog__title">{{ title }}</span>
      <span class="prog__stage">{{ stageText }}</span>
      <span class="prog__detail">{{ detail }}</span>
    </div>

    <div class="prog__bar" role="progressbar" :aria-valuenow="percent ?? undefined" aria-valuemin="0" aria-valuemax="100">
      <!-- 长度未知时用不确定态（横向流动条纹），不要假装有进度 -->
      <div v-if="percent !== null" class="prog__fill" :style="{ width: `${percent}%` }" />
      <div v-else class="prog__indeterminate" />
    </div>

    <div class="prog__foot">
      <span v-if="percent !== null" class="prog__pct">{{ percent }}%</span>
      <span v-else class="prog__pct">长度未知</span>
      <el-button v-if="cancellable" size="small" @click="emit('cancel')">取消</el-button>
    </div>
  </section>
</template>

<style scoped>
.prog {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  padding: var(--sp-3);
  border-radius: var(--r-card);
  background: var(--c-surface-2);
}

.prog__head {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
  flex-wrap: wrap;
}

.prog__title {
  color: var(--c-text);
  font-size: var(--f-size-sm);
  font-weight: 500;
}

.prog__stage {
  padding: 0 var(--sp-2);
  border-radius: var(--r-pill);
  background: var(--c-surface);
  color: var(--c-primary);
  font-size: var(--f-size-xs);
  line-height: 18px;
}

.prog__detail {
  margin-left: auto;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
  font-variant-numeric: tabular-nums;
}

.prog__bar {
  position: relative;
  height: 6px;
  border-radius: var(--r-pill);
  background: var(--c-surface);
  overflow: hidden;
}

.prog__fill {
  height: 100%;
  border-radius: var(--r-pill);
  background: var(--c-primary);
  transition: width 0.2s linear;
}

/* 长度未知：不确定态动画 */
.prog__indeterminate {
  position: absolute;
  inset: 0;
  width: 40%;
  border-radius: var(--r-pill);
  background: var(--c-primary);
  animation: prog-slide 1.2s ease-in-out infinite;
}

@keyframes prog-slide {
  0% {
    transform: translateX(-100%);
  }
  100% {
    transform: translateX(250%);
  }
}

.prog__foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.prog__pct {
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
  font-variant-numeric: tabular-nums;
}
</style>
