<script setup lang="ts">
/**
 * 用量统计 —— 按模型 / 按天的 Token 统计 + CSV 导出。
 *
 * 见 docs/05-项目开发方案.md §2.1.2「用量统计」与 §1.3.2 组 11。
 *
 * ## 一次请求拿三种视图
 *
 * `usage_stats` **一次返回总计 + 按模型 + 按天**（数据量很小，
 * 没必要拆三个命令）。所以「按模型 / 按天」只是**前端切换表格**，
 * 不重新请求 —— 切换是瞬时的。
 *
 * ## CSV 导出的路径必须由用户选
 *
 * `usage_export_csv(path)` 需要一个绝对路径。这里走 `dialog.save`
 * 让用户选 —— 一是符合桌面习惯，二是 Tauri 的 fs scope
 * 只放行用户选过的路径（后端写文件同样受此约束）。
 *
 * ## 清空要二次确认
 *
 * 用量是**不可再生**的历史数据（重算不会补回来），
 * 所以 `usageClear(true)` 前必须问一次。
 */
import { computed, onActivated, ref } from 'vue'
import { usageApi } from '@/api'
import { pickJsonSavePath } from '@/api/dialog'
import TokenStatGrid from '@/components/settings/TokenStatGrid.vue'
import { errorMessage } from '@/types/error'
import type { UsageStats } from '@/types/system'

const stats = ref<UsageStats | null>(null)
const loading = ref(true)
const groupBy = ref<'model' | 'day'>('model')
const exporting = ref(false)


/**
 * 被 `keep-alive` 缓存后**不会重新挂载**，切回来时数据仍是旧快照。
 * 所以每次激活都重新拉一次 —— 否则在别处新增/删除了记录，
 * 这里要等应用重启才看得到。
 */
onActivated(load)

async function load(): Promise<void> {
  loading.value = true
  try {
    stats.value = await usageApi.usageStats(groupBy.value)
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  } finally {
    loading.value = false
  }
}

/** 空态判定：一条调用记录都没有 */
const isEmpty = computed(() => (stats.value?.total.callCount ?? 0) === 0)

/** 表格行（按当前分组口径） */
interface Row {
  key: string
  label: string
  callCount: number
  prompt: number
  completion: number
  total: number
  cached: number
  reasoning: number
}

const rows = computed<Row[]>(() => {
  const s = stats.value
  if (!s) return []
  if (groupBy.value === 'model') {
    return s.byModel.map((m) => ({
      key: m.modelLabel,
      label: m.modelLabel,
      callCount: m.callCount,
      prompt: m.totalPrompt,
      completion: m.totalCompletion,
      total: m.totalAll,
      cached: m.totalCached,
      reasoning: m.totalReasoning,
    }))
  }
  return s.byDay.map((d) => ({
    key: d.day,
    label: d.day,
    callCount: d.callCount,
    prompt: d.totalPrompt,
    completion: d.totalCompletion,
    total: d.totalAll,
    cached: d.totalCached,
    reasoning: d.totalReasoning,
  }))
})

function n(v: number): string {
  return v.toLocaleString('zh-CN')
}

// ---------------------------------------------------------------- 导出

async function exportCsv(): Promise<void> {
  if (exporting.value) return
  const path = await pickJsonSavePath(`civilcalc-usage-${todayStamp()}.csv`)
  if (!path) return

  exporting.value = true
  try {
    const lines = await usageApi.usageExportCsv(path)
    ElMessage.success(`已导出 ${lines} 行（含 UTF-8 BOM，Excel 双击不乱码）`)
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  } finally {
    exporting.value = false
  }
}

function todayStamp(): string {
  const d = new Date()
  const p = (x: number) => String(x).padStart(2, '0')
  return `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}`
}

// ---------------------------------------------------------------- 清空

async function clearAll(): Promise<void> {
  try {
    await ElMessageBox.confirm(
      `将清空全部 ${n(stats.value?.total.callCount ?? 0)} 条用量记录。清空后无法恢复（重新计算不会补回）。`,
      '确认清空用量',
      { type: 'warning', confirmButtonText: '清空', cancelButtonText: '取消' },
    )
  } catch {
    return
  }
  try {
    const removed = await usageApi.usageClear(true)
    ElMessage.success(`已清空 ${removed} 条`)
    await load()
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  }
}
</script>

<template>
  <div class="view">
    <el-skeleton v-if="loading" :rows="6" animated />

    <template v-else-if="stats">
      <!-- 空态 -->
      <section v-if="isEmpty" class="panel">
        <div class="panel__body empty">
          <p class="empty__title">还没有用量记录</p>
          <p class="empty__text">
            每次 AI 解析、详解生成与模型测试都会记一笔（含输入/输出/缓存/思考 token）。
            用一次 AI 功能后回来就能看到。
          </p>
        </div>
      </section>

      <template v-else>
        <!-- 总计 -->
        <section class="panel">
          <div class="panel__head">
            <span class="panel__title">总用量</span>
            <div class="head__ops">
              <el-button size="small" :loading="exporting" @click="exportCsv()">
                导出 CSV
              </el-button>
              <el-button size="small" @click="clearAll()">清空</el-button>
            </div>
          </div>
          <div class="panel__body">
            <TokenStatGrid :totals="stats.total" />
          </div>
        </section>

        <!-- 明细 -->
        <section class="panel">
          <div class="panel__head">
            <span class="panel__title">明细</span>
            <div class="seg">
              <button
                class="seg__btn"
                :class="{ 'seg__btn--on': groupBy === 'model' }"
                type="button"
                @click="groupBy = 'model'"
              >
                按模型
              </button>
              <button
                class="seg__btn"
                :class="{ 'seg__btn--on': groupBy === 'day' }"
                type="button"
                @click="groupBy = 'day'"
              >
                按天
              </button>
            </div>
          </div>

          <div class="panel__body">
            <p v-if="rows.length === 0" class="empty__text">这个维度下还没有数据。</p>

            <table v-else class="tbl">
              <thead>
                <tr>
                  <th class="tbl__left">{{ groupBy === 'model' ? '模型' : '日期' }}</th>
                  <th>调用</th>
                  <th>输入</th>
                  <th>输出</th>
                  <th>缓存</th>
                  <th>思考</th>
                  <th>合计</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="r in rows" :key="r.key">
                  <td class="tbl__left">{{ r.label }}</td>
                  <td>{{ n(r.callCount) }}</td>
                  <td>{{ n(r.prompt) }}</td>
                  <td>{{ n(r.completion) }}</td>
                  <td>{{ n(r.cached) }}</td>
                  <td>{{ n(r.reasoning) }}</td>
                  <td class="tbl__total">{{ n(r.total) }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </section>
      </template>
    </template>
  </div>
</template>

<style scoped>
.view {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  max-width: none;
}

.panel {
  border-radius: var(--r-card);
  background: var(--c-surface);
}

.panel__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--sp-3);
  padding: var(--sp-4) var(--sp-4) 0;
}

.panel__title {
  font-weight: 600;
  color: var(--c-text);
}

.panel__body {
  padding: var(--sp-3) var(--sp-4) var(--sp-4);
}

.head__ops {
  display: flex;
  gap: var(--sp-2);
}

.empty {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  padding: var(--sp-6) var(--sp-4);
  text-align: center;
}

.empty__title {
  margin: 0;
  color: var(--c-text);
  font-size: var(--f-size-lg);
  font-weight: 600;
}

.empty__text {
  margin: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
  line-height: 1.7;
}

/* 分段切换 */
.seg {
  display: inline-flex;
  padding: 2px;
  border-radius: var(--r-btn);
  background: var(--c-surface-2);
}

.seg__btn {
  padding: var(--sp-1) var(--sp-3);
  border: none;
  border-radius: var(--r-sm);
  background: transparent;
  color: var(--c-text-2);
  font-family: inherit;
  font-size: var(--f-size-sm);
  cursor: pointer;
}

.seg__btn--on {
  background: var(--c-surface);
  color: var(--c-primary);
  font-weight: 500;
}

/* 表格：不用全边框，仅横向分割线 */
.tbl {
  width: 100%;
  border-collapse: collapse;
  font-size: var(--f-size-sm);
  font-variant-numeric: tabular-nums;
}

.tbl th,
.tbl td {
  padding: var(--sp-2) var(--sp-3);
  border-bottom: var(--hairline) solid var(--c-divider);
  text-align: right;
  white-space: nowrap;
}

.tbl th {
  background: var(--c-surface-2);
  color: var(--c-text-3);
  font-weight: 500;
}

.tbl__left {
  text-align: left;
}

.tbl__total {
  font-weight: 600;
  color: var(--c-text);
}
</style>
