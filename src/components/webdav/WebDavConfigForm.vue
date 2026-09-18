<script setup lang="ts">
/**
 * WebDavConfigForm —— WebDAV 服务器配置（地址 / 账号 / 密码 / 目录）。
 *
 * 见 docs/05-项目开发方案.md「流程 5」与 docs/08 §3 组 14。
 *
 * ## 🔴 这里的 `password` 是 **WebDAV 账号密码**，不是备份包密码
 *
 * 全项目有**两个同名不同物**的 `password`：
 *
 * | 位置 | 含义 | 存哪儿 |
 * |---|---|---|
 * | 本组件 | 连服务器的账号密码 | 系统凭据管理器（keyring） |
 * | 上传/解析/下载导入 | 备份包加密密码 | **不存**，用完即弃 |
 *
 * 传错不会报错，只表现为「连不上」或「打不开包」，非常难查。
 *
 * ## 密码提交后**立即清空输入框**
 *
 * 契约要求。所以「留空 = 不改动已保存的密码」，而不是「把密码设成空」——
 * 用户下次进来只看到「已保存」状态，不需要（也拿不到）明文。
 *
 * ## 🔴 两类失败必须分开呈现
 *
 * - **本地配置问题**（地址不是 https / 没密码）→ 后端**抛错** → 引导改配置
 * - **连不上**（网络、鉴权被拒、证书）→ 返回 `ok: false` → 提示查网络
 *
 * 都做成一句红字的话，用户不知道该改哪儿。
 *
 * ## 测试连接用的是**表单当前值**，不是已保存的值
 *
 * 正常流程是「填地址 → 测试 → 保存」。若只能测已保存的配置，
 * 用户就得先保存一份可能是错的配置才能测 —— 本末倒置。
 */
import { computed, ref } from 'vue'
import { webdavApi } from '@/api'
import { errorMessage } from '@/types/error'
import {
  defaultBaseUrlOf,
  type WebDavConfig,
  type WebDavPreset,
  type WebDavTestResult,
} from '@/types/backup'

const props = withDefaults(
  defineProps<{
    /** 当前配置（`v-model`） */
    modelValue: WebDavConfig
    /** 是否已保存密码（**不返回密码本身**） */
    hasPassword: boolean
    /** 外部忙碌（上传/下载中禁用） */
    disabled?: boolean
  }>(),
  { disabled: false },
)

const emit = defineEmits<{
  (e: 'update:modelValue', v: WebDavConfig): void
  /** 保存成功后（父组件重载列表与统计） */
  (e: 'saved'): void
  /** 清除密码 */
  (e: 'clear-password'): void
}>()

/** WebDAV 账号密码（**提交后立即清空**） */
const password = ref('')
const testing = ref(false)
const saving = ref(false)
const result = ref<WebDavTestResult | null>(null)

const cfg = computed(() => props.modelValue)

function patch(p: Partial<WebDavConfig>): void {
  emit('update:modelValue', { ...props.modelValue, ...p })
}

/** 切预设时把地址一并换成该预设的默认值 */
function onPreset(preset: WebDavPreset): void {
  patch({ preset, baseUrl: defaultBaseUrlOf(preset) })
}

/** 地址必须是 https —— 源项目与后端都只放行 https（明文会把密码暴露在链路上） */
const urlProblem = computed(() => {
  const u = cfg.value.baseUrl.trim()
  if (!u) return '请填写服务器地址'
  if (!u.toLowerCase().startsWith('https://')) return '只支持 https 地址（明文 http 会暴露账号密码）'
  return ''
})

const canAct = computed(() => !props.disabled && !urlProblem.value)

async function test(): Promise<void> {
  if (!canAct.value || testing.value) return
  testing.value = true
  result.value = null
  try {
    // 用表单当前值测（而不是已保存的配置）
    result.value = await webdavApi.webdavTestConnection(
      cfg.value,
      password.value.trim() || undefined,
    )
  } catch (e) {
    // 配置问题走 Err —— 与「连不通」分开呈现
    const msg = errorMessage(e as never)
    result.value = { ok: false, dirUrl: '', message: msg }
    ElMessage.error(msg)
  } finally {
    testing.value = false
  }
}

async function save(): Promise<void> {
  if (!canAct.value || saving.value) return
  saving.value = true
  try {
    await webdavApi.webdavConfigSave(cfg.value, password.value.trim() || undefined)
    // 契约：密码提交后立即清空
    password.value = ''
    ElMessage.success('已保存')
    emit('saved')
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <div class="wdf">
    <div class="wdf__row">
      <label class="field">
        <span class="field__label">服务商</span>
        <el-select
          :model-value="cfg.preset"
          :disabled="disabled"
          @update:model-value="(v: WebDavPreset) => onPreset(v)"
        >
          <el-option label="坚果云（Nutstore）" value="NUTSTORE" />
          <el-option label="自定义" value="CUSTOM" />
        </el-select>
      </label>

      <label class="field field--wide">
        <span class="field__label">服务器地址 <em>*</em></span>
        <el-input
          :model-value="cfg.baseUrl"
          :disabled="disabled"
          placeholder="https://dav.jianguoyun.com/dav"
          @update:model-value="(v: string) => patch({ baseUrl: v })"
        />
      </label>
    </div>

    <p v-if="urlProblem" class="warn">{{ urlProblem }}</p>

    <div class="wdf__row">
      <label class="field">
        <span class="field__label">账号</span>
        <el-input
          :model-value="cfg.username"
          :disabled="disabled"
          placeholder="登录邮箱"
          @update:model-value="(v: string) => patch({ username: v })"
        />
      </label>

      <label class="field">
        <span class="field__label">密码 / 应用授权码</span>
        <el-input
          v-model="password"
          type="password"
          show-password
          :disabled="disabled"
          :placeholder="hasPassword ? '已保存（留空则不改动）' : '尚未保存'"
        />
      </label>

      <label class="field">
        <span class="field__label">远端目录</span>
        <el-input
          :model-value="cfg.remoteDir"
          :disabled="disabled"
          placeholder="civilcalc"
          @update:model-value="(v: string) => patch({ remoteDir: v })"
        />
      </label>
    </div>

    <p class="hint">
      坚果云要在「账户信息 → 安全选项 → 添加应用密码」里生成授权码，用**登录密码**会 401。
      密码只存进系统凭据管理器，不会随备份包或配置导出。
    </p>

    <div class="wdf__ops">
      <el-button :loading="testing" :disabled="!canAct" @click="test()">测试连接</el-button>
      <el-button type="primary" :loading="saving" :disabled="!canAct" @click="save()">保存</el-button>
      <el-button v-if="hasPassword" :disabled="disabled" @click="emit('clear-password')">
        清除已保存的密码
      </el-button>
    </div>

    <!-- 测试结果 -->
    <div v-if="result" class="res" :class="{ 'res--ok': result.ok, 'res--bad': !result.ok }">
      <span class="res__mark">{{ result.ok ? '✓' : '✕' }}</span>
      <span class="res__msg">{{ result.message || (result.ok ? '连接正常' : '连接失败') }}</span>
      <span v-if="result.dirUrl" class="res__url">{{ result.dirUrl }}</span>
    </div>
  </div>
</template>

<style scoped>
.wdf {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
}

.wdf__row {
  display: flex;
  gap: var(--sp-3);
  flex-wrap: wrap;
}

.field {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
  flex: 1;
  min-width: 160px;
}

.field--wide {
  flex: 2;
}

.field__label {
  color: var(--c-text-2);
  font-size: var(--f-size-sm);
}

.field__label em {
  color: var(--c-danger);
  font-style: normal;
}

.hint {
  margin: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
  line-height: 1.7;
}

.warn {
  margin: 0;
  padding: var(--sp-2) var(--sp-3);
  border-radius: var(--r-sm);
  background: var(--c-surface-2);
  color: var(--c-warning);
  font-size: var(--f-size-xs);
}

.wdf__ops {
  display: flex;
  gap: var(--sp-2);
  flex-wrap: wrap;
}

.res {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  padding: var(--sp-2) var(--sp-3);
  border-radius: var(--r-sm);
  background: var(--c-surface-2);
  font-size: var(--f-size-xs);
  line-height: 1.7;
}

.res--ok {
  color: var(--c-primary);
}

.res--bad {
  color: var(--c-danger);
}

.res__mark {
  flex-shrink: 0;
}

.res__msg {
  flex: 1;
  min-width: 0;
}

.res__url {
  color: var(--c-text-3);
  font-family: var(--f-mono);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 45%;
}
</style>
