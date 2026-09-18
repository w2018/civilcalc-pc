<script setup lang="ts">
/**
 * ModelProfileForm —— 模型档位的编辑弹窗（含密钥）。
 *
 * 见 docs/05-项目开发方案.md §2.1.2「设置」与 §1.3.2 组 8。
 *
 * ## 🔒 密钥的铁律
 *
 * - `LlmProfile` **结构里没有 apiKey 字段**（后端也不返回），
 *   所以「是否已设置」只能靠 `llm_has_api_key` 单独查，用占位文案表达
 * - 提交后**立刻清空**输入框里的明文，不留在内存里
 * - 留空 = **不动已有密钥**（`llmSaveProfile(profile, undefined)`），
 *   而不是「把密钥删掉」—— 想删要走列表里的「删除密钥」
 *
 * ## `baseUrl` 要填到含版本号的目录
 *
 * 源项目踩过的坑：只填 `https://api.deepseek.com` 会 404。
 * 所以下面挂了明确的提示文案，而不是等用户撞墙。
 *
 * ## 思考强度与厂商能力的冲突要在**这里**提示
 *
 * 后端会把 GLM 的「关闭」降级为「低」、把 MiMo 的档位忽略。
 * 如果不在选项旁说明，用户会以为设置没生效。
 * 判定逻辑复用 `types/llm.ts` 的三个函数（与后端同一套规则）。
 */
import { computed, ref, watch } from 'vue'
import {
  API_PROTOCOL_LABELS,
  THINKING_LEVEL_LABELS,
  thinkingLevelHint,
  supportsWebSearch,
  type ApiProtocol,
  type LlmProfile,
  type ThinkingLevel,
} from '@/types/llm'

const props = withDefaults(
  defineProps<{
    /** 是否显示（`v-model`） */
    modelValue: boolean
    /** 正在编辑的档位；新建时为 `null` */
    profile: LlmProfile | null
    /** 是否新建 */
    creating?: boolean
    /** 该档位是否已存有密钥 */
    hasKey?: boolean
  }>(),
  { creating: false, hasKey: false },
)

const emit = defineEmits<{
  (e: 'update:modelValue', v: boolean): void
  /** 提交：`apiKey` 为 `undefined` 表示不动已有密钥 */
  (e: 'submit', profile: LlmProfile, apiKey?: string): void
  /** 删掉这一档已保存的密钥（需求 3：密钥的增删都收进这个弹窗） */
  (e: 'delete-key', profile: LlmProfile): void
}>()

/**
 * 删除已保存的密钥。
 *
 * 必须二次确认：删掉之后要重新去厂商后台复制一遍 Key，
 * 而这里点一下是不可撤销的。
 */
async function removeKey(): Promise<void> {
  const p = props.profile
  if (!p) return
  try {
    await ElMessageBox.confirm(
      `会从系统凭据管理器里删掉「${p.label}」的 API Key。删掉后这一档在补回 Key 之前无法使用。`,
      '删除已保存的密钥？',
      { type: 'warning', confirmButtonText: '删除密钥', cancelButtonText: '取消' },
    )
  } catch {
    return // 用户取消
  }
  emit('delete-key', p)
}

/** 表单副本（不直接改 props） */
const form = ref<LlmProfile>(blank())

function blank(): LlmProfile {
  return {
    id: '',
    label: '',
    baseUrl: '',
    model: '',
    enabled: true,
    webSearch: false,
    thinkingLevel: 'AUTO',
    vision: true,
    apiProtocol: 'AUTO',
  }
}

/** 密钥输入（明文只活在这个 ref 里，提交后立刻清） */
const apiKey = ref('')

watch(
  () => props.modelValue,
  (open) => {
    if (!open) return
    apiKey.value = ''
    form.value = props.profile ? { ...props.profile } : blank()
  },
)

const thinkingOptions = Object.keys(THINKING_LEVEL_LABELS) as ThinkingLevel[]
const protocolOptions = Object.keys(API_PROTOCOL_LABELS) as ApiProtocol[]

/** 当前 baseUrl + 档位下的降级提示（没有则 `null`） */
const levelHint = computed(() => thinkingLevelHint(form.value.baseUrl, form.value.thinkingLevel))

/** 该厂商是否支持联网（不支持时把开关禁掉，而不是让后端报错） */
const webSearchAvailable = computed(() => supportsWebSearch(form.value.baseUrl))

const issues = computed<string[]>(() => {
  const f = form.value
  const out: string[] = []
  if (!f.label.trim()) out.push('显示名不能为空')
  if (!f.baseUrl.trim()) out.push('接口地址不能为空')
  else if (!/^https?:\/\//i.test(f.baseUrl.trim())) out.push('接口地址要以 http(s):// 开头')
  if (!f.model.trim()) out.push('模型名不能为空')
  if (!props.creating && !f.id.trim()) out.push('档位 id 缺失')
  return out
})

function submit(): void {
  if (issues.value.length > 0) {
    ElMessage.warning(issues.value[0])
    return
  }
  const f = form.value
  const out: LlmProfile = {
    ...f,
    // 新建时用「时间戳」做 id：内置三家用的是固定名（deepseek/mimo/glm），
    // 自建档位不能撞上它们
    id: f.id.trim() || `custom-${Date.now().toString(36)}`,
    label: f.label.trim(),
    baseUrl: f.baseUrl.trim().replace(/\/+$/, ''),
    model: f.model.trim(),
  }
  const key = apiKey.value.trim()
  emit('submit', out, key ? key : undefined)
  apiKey.value = ''
  emit('update:modelValue', false)
}
</script>

<template>
  <el-dialog
    :model-value="modelValue"
    :title="creating ? '添加模型档位' : '编辑模型档位'"
    width="560px"
    :close-on-click-modal="false"
    @update:model-value="(v: boolean) => emit('update:modelValue', v)"
  >
    <div class="form">
      <label class="field">
        <span class="field__label">显示名 <em>*</em></span>
        <el-input v-model="form.label" placeholder="如：DeepSeek 官方" />
      </label>

      <label class="field">
        <span class="field__label">接口地址 <em>*</em></span>
        <el-input v-model="form.baseUrl" placeholder="https://api.deepseek.com/v1" />
        <span class="field__hint">
          要填到**含版本号的目录**（如 <code>/v1</code>、<code>/api/paas/v4</code>）。
          只填域名会 404。
        </span>
      </label>

      <label class="field">
        <span class="field__label">模型名 <em>*</em></span>
        <el-input v-model="form.model" placeholder="如：deepseek-chat" />
      </label>

      <label class="field">
        <span class="field__label">API Key</span>
        <el-input
          v-model="apiKey"
          type="password"
          show-password
          :placeholder="hasKey ? '已设置（留空则不改动）' : '尚未设置'"
        />
        <span class="field__hint">
          Key 只存进 Windows 凭据管理器，**不会**随配置导出（除非你显式选择导出密钥）。
        </span>
        <!-- 密钥的删除入口在这里（需求 3）—— 列表行上不再重复放一个 -->
        <button
          v-if="hasKey && !creating"
          class="link-danger"
          type="button"
          @click="removeKey()"
        >
          删除已保存的密钥
        </button>
      </label>

      <div class="row">
        <label class="field">
          <span class="field__label">思考强度</span>
          <el-select v-model="form.thinkingLevel">
            <el-option
              v-for="t in thinkingOptions"
              :key="t"
              :label="THINKING_LEVEL_LABELS[t]"
              :value="t"
            />
          </el-select>
        </label>

        <label class="field">
          <span class="field__label">接入协议</span>
          <el-select v-model="form.apiProtocol">
            <el-option
              v-for="p in protocolOptions"
              :key="p"
              :label="API_PROTOCOL_LABELS[p]"
              :value="p"
            />
          </el-select>
        </label>
      </div>

      <p v-if="levelHint" class="warn">{{ levelHint }}</p>

      <div class="checks">
        <el-checkbox v-model="form.enabled">启用（可被选为活跃档位）</el-checkbox>
        <el-checkbox v-model="form.vision">支持图片识别</el-checkbox>
        <el-checkbox v-model="form.webSearch" :disabled="!webSearchAvailable">
          联网搜索{{ webSearchAvailable ? '' : '（该厂商不支持）' }}
        </el-checkbox>
      </div>

      <ul v-if="issues.length > 0" class="issues">
        <li v-for="(m, i) in issues" :key="i">{{ m }}</li>
      </ul>
    </div>

    <template #footer>
      <el-button @click="emit('update:modelValue', false)">取消</el-button>
      <el-button type="primary" @click="submit()">保存</el-button>
    </template>
  </el-dialog>
</template>

<style scoped>
.form {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
}

.field {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
  flex: 1;
  min-width: 0;
}

.field__label {
  font-size: var(--f-size-sm);
  color: var(--c-text-2);
}

.field__label em {
  color: var(--c-danger);
  font-style: normal;
}

.field__hint {
  font-size: var(--f-size-xs);
  color: var(--c-text-3);
  line-height: 1.6;
}

.field__hint code {
  font-family: var(--f-mono);
}

/* 「删除已保存的密钥」：低调的文字按钮，靠左对齐（不是主操作） */
.link-danger {
  align-self: flex-start;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--c-danger);
  font-family: inherit;
  font-size: var(--f-size-xs);
  cursor: pointer;
}

.link-danger:hover {
  text-decoration: underline;
}

.row {
  display: flex;
  gap: var(--sp-3);
}

.warn {
  margin: 0;
  padding: var(--sp-2) var(--sp-3);
  border-radius: var(--r-sm);
  background: var(--c-surface-2);
  color: var(--c-warning);
  font-size: var(--f-size-xs);
}

.checks {
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-3);
}

.issues {
  margin: 0;
  padding-left: var(--sp-4);
  color: var(--c-warning);
  font-size: var(--f-size-sm);
}
</style>
