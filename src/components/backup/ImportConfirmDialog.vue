<script setup lang="ts">
/**
 * ImportConfirmDialog —— 导入前的二次确认（本地包与云端包**共用**）。
 *
 * 见 docs/05-项目开发方案.md「流程 5：备份与恢复」与 docs/08 §3 组 14。
 *
 * ## 数字来自包本身，不是估算
 *
 * 弹窗展示的摘要来自 `backup_inspect` / `webdav_inspect` 解析出的
 * **清单**（不是「本机会变成什么样」）。所以文案是「这个包里有什么」，
 * 不要写成「将导入 N 条」—— 合并模式下实际写入数可能更少
 * （同 ID 记录会被覆盖）。
 *
 * ## 🔴 加密包没密码时 `decoded === false` 是**正常返回**
 *
 * 这时不报错、也不允许直接导入 —— 就地让用户填密码并**重新 inspect**。
 * 关键是：`webdav_inspect` 已经把包下载到临时目录了，
 * 重新 inspect 同一个包**不会重新下载**（几十 MB 不传两遍）。
 *
 * ## `replace` 的措辞要精确
 *
 * 它**只清空包里确实含有的类别**。写「清空本机全部数据」是错的 ——
 * 那会让用户以为导入一个只含公式的包会连历史一起清掉。
 */
import { computed, ref, watch } from 'vue'
import { formatDateTime } from '@/utils/format'
import {
  describeSummary,
  humanSize,
  isSummaryEmpty,
  sectionLabel,
  type BackupInspectResult,
  type ImportMode,
} from '@/types/backup'

const props = withDefaults(
  defineProps<{
    /** 是否显示（`v-model`） */
    modelValue: boolean
    /** inspect 的结果；为 `null` 时不渲染内容 */
    inspect: BackupInspectResult | null
    /** 来源（只影响文案：「本机文件」/「云端备份」） */
    source?: 'local' | 'cloud'
    /** 导入进行中 */
    busy?: boolean
  }>(),
  { source: 'local', busy: false },
)

const emit = defineEmits<{
  (e: 'update:modelValue', v: boolean): void
  /**
   * 用户确认导入。
   *
   * ⚠️ **不带密码** —— 加密包的密码由父组件持有：
   * 它才是「这次 inspect 是用哪个密码解开的」的唯一知情人。
   * 让本组件传密码会踩到「重试成功后再点导入，密码已被清空」的坑。
   */
  (e: 'confirm', mode: ImportMode): void
  /** 加密包解不开时：带密码重新 inspect */
  (e: 'retry-password', password: string): void
}>()

/** 导入口径：默认 merge（ADR-021） */
const mode = ref<ImportMode>('merge')
/** 加密包密码（**用完即清**；父组件在 inspect 成功后另行持有） */
const password = ref('')

watch(
  () => props.modelValue,
  (open) => {
    if (open) {
      mode.value = 'merge'
      password.value = ''
    }
  },
)

/** 加密但没解出清单 → 要先给密码 */
const needPassword = computed(
  () => props.inspect !== null && props.inspect.encrypted && !props.inspect.decoded,
)

/** 能直接导入吗（已解出清单，且至少含一个类别） */
const canImport = computed(
  () => props.inspect !== null && props.inspect.decoded && props.inspect.sections.length > 0,
)

const summaryText = computed(() => {
  const s = props.inspect?.summary
  if (!s) return ''
  return describeSummary(s)
})

const emptyPackage = computed(() => (props.inspect ? isSummaryEmpty(props.inspect.summary) : false))

// 时间显示统一走 `utils/format` 的 `formatDateTime`（完整年月日时分秒）
const createdAtText = computed(() => {
  const ms = props.inspect?.createdAt ?? 0
  if (!ms) return '未知'
  return formatDateTime(ms)
})

function onRetry(): void {
  const p = password.value.trim()
  if (!p) {
    ElMessage.warning('请输入备份包密码')
    return
  }
  emit('retry-password', p)
  password.value = ''
}

function onConfirm(): void {
  // 密码不在事件里传 —— 加密包在 inspect 成功后，父组件已经持有那个密码。
  // 这里传会把「重试成功 → 再点导入」这条路径上的密码丢掉。
  emit('confirm', mode.value)
  password.value = ''
}
</script>

<template>
  <el-dialog
    :model-value="modelValue"
    :title="source === 'cloud' ? '导入云端备份' : '导入本机备份'"
    width="560px"
    :close-on-click-modal="false"
    @update:model-value="(v: boolean) => emit('update:modelValue', v)"
  >
    <div v-if="inspect" class="dlg">
      <!-- 包信息 -->
      <div class="meta">
        <div class="meta__row">
          <span class="meta__key">文件</span>
          <span class="meta__val meta__val--mono">{{ inspect.fileName }}</span>
        </div>
        <div class="meta__row">
          <span class="meta__key">大小</span>
          <span class="meta__val">{{ humanSize(inspect.sizeBytes) }}</span>
        </div>
        <div class="meta__row">
          <span class="meta__key">打包时间</span>
          <span class="meta__val">{{ createdAtText }}</span>
        </div>
        <div class="meta__row">
          <span class="meta__key">应用版本</span>
          <span class="meta__val">{{ inspect.appVersionName || '未知' }}</span>
        </div>
        <div class="meta__row">
          <span class="meta__key">加密</span>
          <span class="meta__val">{{ inspect.encrypted ? '是' : '否' }}</span>
        </div>
      </div>

      <!-- 需要密码 -->
      <template v-if="needPassword">
        <p class="lead">
          这是一个**加密**备份包，需要密码才能看到内容。密码只用于这次解密，不会被保存。
        </p>
        <label class="field">
          <span class="field__label">备份包密码</span>
          <el-input
            v-model="password"
            type="password"
            show-password
            placeholder="请输入备份包密码"
            @keyup.enter="onRetry()"
          />
        </label>
        <p class="hint">
          密码不对时会提示「密码错误」，**不需要重新下载**（包已在临时目录里）。
        </p>
      </template>

      <!-- 已解出清单 -->
      <template v-else-if="canImport">
        <p class="lead">这个包里包含：</p>
        <ul class="sections">
          <li v-for="s in inspect.sections" :key="s">{{ sectionLabel(s) }}</li>
        </ul>
        <p v-if="summaryText" class="summary">合计：{{ summaryText }}</p>

        <div class="modes">
          <label class="mode" :class="{ 'mode--on': mode === 'merge' }">
            <input v-model="mode" type="radio" value="merge" />
            <span class="mode__main">
              <span class="mode__title">合并（推荐）</span>
              <span class="mode__hint">按 ID 覆盖同名记录，本机独有的数据保留</span>
            </span>
          </label>
          <label class="mode" :class="{ 'mode--on': mode === 'replace' }">
            <input v-model="mode" type="radio" value="replace" />
            <span class="mode__main">
              <span class="mode__title">替换</span>
              <span class="mode__hint">
                先清空**这个包里含有的类别**再写入 —— 包里没带的类别（本机历史等）保持原样
              </span>
            </span>
          </label>
        </div>

        <p v-if="mode === 'replace'" class="warn">
          替换口径会先清空上列类别，**不可撤销**。若不确定，用「合并」。
        </p>
        <p v-if="emptyPackage" class="warn">这个包是空的，导入不会有任何变化。</p>
      </template>

      <!-- 解不开且不是密码问题（理论上不该出现） -->
      <p v-else class="warn">无法解析这个备份包的清单。</p>
    </div>

    <template #footer>
      <el-button :disabled="busy" @click="emit('update:modelValue', false)">取消</el-button>
      <el-button v-if="needPassword" type="primary" :disabled="busy" @click="onRetry()">
        用这个密码查看
      </el-button>
      <el-button
        v-else
        type="primary"
        :loading="busy"
        :disabled="!canImport || emptyPackage"
        @click="onConfirm()"
      >
        开始导入
      </el-button>
    </template>
  </el-dialog>
</template>

<style scoped>
.dlg {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
}

.meta {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--sp-1) var(--sp-4);
  padding: var(--sp-3);
  border-radius: var(--r-card);
  background: var(--c-surface-2);
}

.meta__row {
  display: flex;
  gap: var(--sp-2);
  min-width: 0;
}

.meta__key {
  flex-shrink: 0;
  width: 60px;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.meta__val {
  color: var(--c-text);
  font-size: var(--f-size-xs);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.meta__val--mono {
  font-family: var(--f-mono);
}

.lead {
  margin: 0;
  color: var(--c-text-2);
  font-size: var(--f-size-sm);
}

.sections {
  margin: 0;
  padding-left: var(--sp-5);
  color: var(--c-text);
  font-size: var(--f-size-sm);
  line-height: 1.9;
}

.summary {
  margin: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.modes {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
}

.mode {
  display: flex;
  align-items: flex-start;
  gap: var(--sp-2);
  padding: var(--sp-2) var(--sp-3);
  border: var(--hairline) solid var(--c-divider);
  border-radius: var(--r-card);
  cursor: pointer;
}

.mode--on {
  border-color: var(--c-primary);
  background: var(--c-primary-light);
}

.mode__main {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.mode__title {
  color: var(--c-text);
  font-size: var(--f-size-sm);
  font-weight: 500;
}

.mode__hint {
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
  line-height: 1.6;
}

.field {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
}

.field__label {
  color: var(--c-text-2);
  font-size: var(--f-size-sm);
}

.hint {
  margin: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.warn {
  margin: 0;
  padding: var(--sp-2) var(--sp-3);
  border-radius: var(--r-sm);
  background: var(--c-surface-2);
  color: var(--c-warning);
  font-size: var(--f-size-xs);
  line-height: 1.7;
}
</style>
