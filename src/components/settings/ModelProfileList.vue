<script setup lang="ts">
/**
 * ModelProfileList —— 模型档位列表（选活跃 / 编辑 / 删 / 密钥 / 测试连接）。
 *
 * 见 docs/05-项目开发方案.md §2.1.2「设置」与 §1.3.2 组 8。
 *
 * ## 🔒 「是否已设置密钥」必须单独查
 *
 * `llm_list_profiles` 的返回**不含 apiKey**（结构里就没这个字段）。
 * 所以「已设置 / 未设置」由父组件调 `llm_has_api_key` 查出来传进来。
 * 不要在列表里假装知道密钥内容。
 *
 * ## 🔴 密钥的增删**不在这一行**（需求 3）
 *
 * 这里曾经还有「设置/更换密钥」与「删除密钥」两个按钮，
 * 而它们要操作的东西在**编辑弹窗**里已经有了 —— 同一件事两个入口，
 * 还让这一行挤了 5 个按钮（测试连接 / 更换密钥 / 删除密钥 / 编辑 / 删除）。
 *
 * 现在：**改密钥、删密钥都在编辑弹窗里**（`ModelProfileForm` 的 API Key 一栏），
 * 列表只保留「这一档是什么状态」与「对它整体做什么」
 * （测试连接 / 编辑 / 删除档位）。
 *
 * ## 删除活跃档位会**自动回落**
 *
 * 后端 `llm_delete_profile` 删掉活跃档位时会自动选一个可用的。
 * 但删除是破坏性操作（**连系统凭据里的 Key 一起删**），
 * 所以这里必须二次确认并写清「会删掉密钥」。
 */
import { computed } from 'vue'
import type { LlmConfig, LlmProfile, LlmTestResult } from '@/types/llm'

const props = defineProps<{
  config: LlmConfig
  /** 档位 id → 是否已存有密钥 */
  keyStatus: Record<string, boolean>
  /** 正在测试连接的档位 id */
  testingId?: string | null
  /** 最近一次测试结果（按档位 id） */
  testResults?: Record<string, LlmTestResult>
}>()

const emit = defineEmits<{
  (e: 'set-active', id: string): void
  (e: 'create'): void
  (e: 'edit', profile: LlmProfile): void
  (e: 'remove', profile: LlmProfile): void
  (e: 'test', profile: LlmProfile): void
}>()

const profiles = computed(() => props.config.profiles)

function hasKey(id: string): boolean {
  return props.keyStatus[id] === true
}

function isActive(p: LlmProfile): boolean {
  return props.config.active === p.id
}

function resultOf(id: string): LlmTestResult | null {
  return props.testResults?.[id] ?? null
}

/** 协议与思考强度的展示文案（省得列表里只有内部枚举名） */
function metaLine(p: LlmProfile): string {
  const bits = [p.model]
  if (p.thinkingLevel !== 'AUTO') bits.push(`思考：${p.thinkingLevel}`)
  if (p.webSearch) bits.push('联网')
  if (p.vision) bits.push('视觉')
  if (p.apiProtocol !== 'AUTO') bits.push(p.apiProtocol)
  return bits.join(' · ')
}
</script>

<template>
  <div class="list">
    <div v-for="p in profiles" :key="p.id" class="row" :class="{ 'row--active': isActive(p) }">
      <!-- 活跃选择（需求 7：CSS 画的单选圆点，热区更大） -->
      <button
        class="row__radio"
        type="button"
        :aria-pressed="isActive(p)"
        :aria-label="`设为活跃档位 ${p.label}`"
        :disabled="!p.enabled"
        @click="emit('set-active', p.id)"
      >
        <span class="row__dot" aria-hidden="true" />
      </button>

      <div class="row__main">
        <div class="row__title">
          <span class="row__label">{{ p.label || '（未命名）' }}</span>
          <span v-if="isActive(p)" class="tag tag--on">活跃</span>
          <span v-if="!p.enabled" class="tag">已禁用</span>
          <span v-if="hasKey(p.id)" class="tag tag--key">已设密钥</span>
          <span v-else class="tag tag--warn">缺密钥</span>
        </div>
        <div class="row__meta">{{ metaLine(p) }}</div>
        <div class="row__url">{{ p.baseUrl }}</div>
        <p v-if="resultOf(p.id)" class="row__test" :class="{ 'row__test--bad': !resultOf(p.id)!.ok }">
          {{ resultOf(p.id)!.ok ? '✓' : '✕' }} {{ resultOf(p.id)!.message }}
          <span v-if="resultOf(p.id)!.latencyMs > 0">（{{ resultOf(p.id)!.latencyMs }} ms）</span>
        </p>
      </div>

      <div class="row__ops">
        <button class="link" type="button" :disabled="testingId === p.id" @click="emit('test', p)">
          {{ testingId === p.id ? '测试中…' : '测试连接' }}
        </button>
        <button class="link" type="button" @click="emit('edit', p)">编辑</button>
        <button class="link link--danger" type="button" @click="emit('remove', p)">删除</button>
      </div>
    </div>

    <div class="list__foot">
      <el-button @click="emit('create')">添加档位</el-button>
    </div>
  </div>
</template>

<style scoped>
.list {
  display: flex;
  flex-direction: column;
}

.row {
  display: flex;
  align-items: flex-start;
  gap: var(--sp-3);
  padding: var(--sp-3) 0;
  border-bottom: var(--hairline) solid var(--c-divider);
}

.row:last-of-type {
  border-bottom: none;
}

/**
 * 当前活跃档位的「选中」控件（需求 7：原来是 `●` / `○` 字符，太小、观感差）。
 *
 * 改成 CSS 画的标准单选圆点：
 * - 点击热区 34×34（原来只有 24×24 且没有 hover 反馈）
 * - 选中态用 `inset box-shadow` 画内圈实心点，不需要额外元素
 * - 用 `aria-pressed` 驱动样式，语义与视觉同源
 */
.row__radio {
  flex-shrink: 0;
  display: grid;
  place-items: center;
  width: 34px;
  height: 34px;
  padding: 0;
  border: none;
  border-radius: 50%;
  background: transparent;
  cursor: pointer;
  transition: background 0.12s ease;
}

.row__radio:hover:not(:disabled) {
  background: var(--c-surface-hover);
}

.row__radio:disabled {
  cursor: default;
  opacity: 0.5;
}

.row__dot {
  width: 18px;
  height: 18px;
  border-radius: 50%;
  border: 2px solid var(--c-divider);
  background: var(--c-surface);
  transition:
    border-color 0.12s ease,
    box-shadow 0.12s ease;
}

.row__radio:hover:not(:disabled) .row__dot {
  border-color: var(--c-text-3);
}

.row__radio[aria-pressed='true'] .row__dot {
  border-color: var(--c-primary);
  /* 内圈实心点：4px 内阴影 = 半径 9 - 2(边框) - 3(留白) */
  box-shadow: inset 0 0 0 4px var(--c-primary);
}

.row__main {
  flex: 1;
  min-width: 0;
}

.row__title {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--sp-2);
}

.row__label {
  font-weight: 500;
  color: var(--c-text);
}

.tag {
  padding: 0 var(--sp-2);
  border-radius: var(--r-pill);
  background: var(--c-surface-2);
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
  line-height: 18px;
}

.tag--on {
  background: var(--c-primary-light);
  color: var(--c-primary);
}

.tag--key {
  background: var(--c-primary-light);
  color: var(--c-primary);
}

.tag--warn {
  background: var(--c-surface-2);
  color: var(--c-warning);
}

.row__meta {
  margin-top: 2px;
  color: var(--c-text-2);
  font-size: var(--f-size-sm);
}

.row__url {
  margin-top: 2px;
  color: var(--c-text-3);
  font-family: var(--f-mono);
  font-size: var(--f-size-xs);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.row__test {
  margin: var(--sp-1) 0 0;
  color: var(--c-primary);
  font-size: var(--f-size-xs);
}

.row__test--bad {
  color: var(--c-danger);
}

.row__ops {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: var(--sp-1);
  flex-shrink: 0;
}

.list__foot {
  margin-top: var(--sp-3);
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
