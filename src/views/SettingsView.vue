<script setup lang="ts">
/**
 * 设置 —— 模型 / 密钥 / AI 行为 / 外观 / 导出选项 / 配置迁移 / 重置。
 *
 * 见 docs/05-项目开发方案.md §2.1.2「设置」与「流程 6：模型配置与密钥」。
 *
 * ## 为什么所有设置都挤在一个页面
 *
 * 桌面应用的设置页是「低频但必须能找到」的地方。
 * 拆成二级页面会增加导航层级，反而不如一个长页面 + 分节标题好找。
 * 每节之间用卡片隔开，视觉上就是一屏一屏的设置项。
 *
 * ## 🔴 `config_save` 是**整体替换**
 *
 * 提示词这类走偏好文件的项，必须「先 `configGet` → 改 → `configSave`」。
 * 只传改动的键会把其它偏好（主题、背景、导出选项…）全清掉。
 * 所以这里持有完整的 `snapshot`，改动时整体提交。
 *
 * ## 🔴 导出配置默认**不含密钥**
 *
 * `llmConfigExport(true)` 会吐出明文 Key。默认传 `false`，
 * 只有用户显式勾选并二次确认才传 `true`。
 *
 * ## 重置后要重载
 *
 * `ResetPanel` 完成时会发 `done` —— 公式、外观、模型配置都可能被清了，
 * 所以这里要重新拉一遍（尤其外观要重新应用，否则界面还留着旧背景）。
 */
import { computed, onDeactivated, onMounted, ref } from 'vue'
import { llmApi, reportApi, systemApi } from '@/api'
import { pickJsonFile, pickJsonSavePath } from '@/api/dialog'
import { readTextFile, saveTextToFile } from '@/api/asset'
import ModelProfileList from '@/components/settings/ModelProfileList.vue'
import ModelProfileForm from '@/components/settings/ModelProfileForm.vue'
import PromptEditor from '@/components/settings/PromptEditor.vue'
import ExportOptionsForm from '@/components/settings/ExportOptionsForm.vue'
import AppearancePanel from '@/components/settings/AppearancePanel.vue'
import ResetPanel from '@/components/settings/ResetPanel.vue'
import AppLogo from '@/components/common/AppLogo.vue'
import { useUiStore } from '@/stores/ui'
import { useFormulaStore } from '@/stores/formula'
import { useActiveModel } from '@/composables/useActiveModel'
import { useThinkingStore } from '@/stores/thinking'
import { errorMessage } from '@/types/error'
import { notifySaved } from '@/utils/toast'
import { DEFAULT_EXPORT_OPTIONS, type ExportOptions } from '@/types/report'
import { PREF_KEYS, type ConfigSnapshot } from '@/types/system'
import type { LlmConfig, LlmProfile, LlmTestResult } from '@/types/llm'

const ui = useUiStore()
/** 活跃档位的全局共享状态（需求 7） */
const { applyConfig, setActiveId, resetActiveModel, refreshActiveModel } = useActiveModel()
/** 刚生成出来的思考暂存（重置时要一并清掉） */
const thinkingStore = useThinkingStore()
/** 公式工作台的占用标记（重置后要清，见 onResetDone） */
const formulaStore = useFormulaStore()

// ---------------------------------------------------------------- 状态

const loading = ref(true)

/** 模型配置（**不含密钥**） */
const config = ref<LlmConfig>({ profiles: [], active: '' })
/** 档位 id → 是否已存有密钥 */
const keyStatus = ref<Record<string, boolean>>({})
const testingId = ref<string | null>(null)
const testResults = ref<Record<string, LlmTestResult>>({})

/** 偏好快照（提示词等） */
const snapshot = ref<ConfigSnapshot>({ strings: {}, ints: {} })
const exportOptions = ref<ExportOptions>({ ...DEFAULT_EXPORT_OPTIONS })

/** 档位编辑弹窗 */
const formOpen = ref(false)
const editing = ref<LlmProfile | null>(null)
const creating = ref(false)
const editingHasKey = ref(false)

// ---------------------------------------------------------------- 分块折叠

/**
 * 设置页各分块的折叠状态（`true` = 收起）。
 *
 * ## 默认**全部收起**
 *
 * 六个分块内容都很长，全展开要滚很久才能翻到想改的那一项。
 * 默认收起 → 一屏看全目录，点哪块展哪块。
 *
 * ## 为什么放 localStorage 而不是偏好文件
 *
 * 与侧栏折叠同理（见 `stores/ui.ts`）：这是**纯 UI 临时状态**，
 * 不是用户可见的「设置」。进偏好会牵动「重置外观」等区块语义，
 * 而且每折一次都写一次盘没必要。
 *
 * ⚠️ 因此「重置软件」**不会**把它清回默认 —— 它本来就不在偏好里。
 */
const COLLAPSE_KEY = 'civilcalc.ui.settingsCollapsed'

/** 分块 key → 是否收起。**只存「用户改过的」**，没记录的按收起算 */
const collapsed = ref<Record<string, boolean>>(readCollapsed())

function readCollapsed(): Record<string, boolean> {
  try {
    const raw = localStorage.getItem(COLLAPSE_KEY)
    if (!raw) return {}
    const parsed: unknown = JSON.parse(raw)
    // 脏数据（被人手改过 / 旧版本格式）一律忽略，回落到「全部收起」
    return parsed && typeof parsed === 'object' ? (parsed as Record<string, boolean>) : {}
  } catch {
    // 隐私模式 / 存储被禁：静默回落，不影响启动
    return {}
  }
}

/** 某个分块是否收起（**没记录过 = 收起**，这样新增分块天然是默认收起的） */
function isCollapsed(key: string): boolean {
  return collapsed.value[key] !== false
}

function toggleSection(key: string): void {
  collapsed.value = { ...collapsed.value, [key]: !isCollapsed(key) }
  try {
    localStorage.setItem(COLLAPSE_KEY, JSON.stringify(collapsed.value))
  } catch {
    // 写不进去也不该报错
  }
}

onMounted(load)

async function load(): Promise<void> {
  loading.value = true
  await Promise.all([loadLlm(), loadPrefs(), loadExportOptions()])
  loading.value = false
}

async function loadLlm(): Promise<void> {
  try {
    config.value = await llmApi.llmListProfiles()
    // 档位可能被增删改（label / 活跃项都会变）→ 推给全局共享状态（需求 7）
    applyConfig(config.value)
    // 密钥状态要**逐个**查（结构里没有 apiKey 字段）
    const entries = await Promise.all(
      config.value.profiles.map(async (p) => {
        try {
          return [p.id, await llmApi.llmHasApiKey(p.id)] as const
        } catch {
          return [p.id, false] as const
        }
      }),
    )
    keyStatus.value = Object.fromEntries(entries)
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  }
}

async function loadPrefs(): Promise<void> {
  try {
    snapshot.value = await systemApi.configGet()
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  }
}

async function loadExportOptions(): Promise<void> {
  try {
    exportOptions.value = await reportApi.exportOptionsGet()
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  }
}

// ---------------------------------------------------------------- 模型档位

function onCreate(): void {
  creating.value = true
  editing.value = null
  editingHasKey.value = false
  formOpen.value = true
}

function onEdit(p: LlmProfile): void {
  creating.value = false
  editing.value = p
  editingHasKey.value = keyStatus.value[p.id] === true
  formOpen.value = true
}

async function onSubmit(profile: LlmProfile, apiKey?: string): Promise<void> {
  try {
    if (creating.value) {
      await llmApi.llmSaveProfile(profile, apiKey)
      ElMessage.success('已添加档位')
    } else if (apiKey !== undefined) {
      // 带了新密钥 → 走 save_profile（它会同时写配置与 Key）
      await llmApi.llmSaveProfile(profile, apiKey)
      ElMessage.success('已保存（含新密钥）')
    } else {
      // 不带密钥 → 走 update_profile（**明确不动已有 Key**）
      await llmApi.llmUpdateProfile(profile)
      ElMessage.success('已保存')
    }
    await loadLlm()
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  }
}

async function onSetActive(id: string): Promise<void> {
  try {
    await llmApi.llmSetActive(id)
    config.value = { ...config.value, active: id }
    // 需求 7：立刻同步给所有页面 —— 否则「新建公式」「模型测试」
    // 还显示着上一个模型（它们被 keep-alive 缓存，不会自己重拉）
    setActiveId(id)
    ElMessage.success('已切换活跃档位')
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  }
}

async function onRemove(p: LlmProfile): Promise<void> {
  try {
    await ElMessageBox.confirm(
      `删除「${p.label}」会同时删掉它在系统凭据管理器里的 API Key，且不可撤销。`,
      '确认删除档位',
      { type: 'warning', confirmButtonText: '删除', cancelButtonText: '取消' },
    )
  } catch {
    return // 用户取消
  }
  try {
    await llmApi.llmDeleteProfile(p.id)
    ElMessage.success('已删除')
    await loadLlm()
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  }
}

/**
 * 删除密钥（入口在**编辑弹窗**里，见 `ModelProfileForm`）。
 *
 * ⚠️ 删完要同步 `editingHasKey`：弹窗还开着，不更新的话
 * 「删除已保存的密钥」按钮不会消失、输入框的占位符也还写着「已设置」。
 */
async function onDeleteKey(p: LlmProfile): Promise<void> {
  try {
    await llmApi.llmDeleteApiKey(p.id)
    keyStatus.value = { ...keyStatus.value, [p.id]: false }
    if (editing.value?.id === p.id) editingHasKey.value = false
    ElMessage.success('已删除密钥')
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  }
}

async function onTest(p: LlmProfile): Promise<void> {
  testingId.value = p.id
  try {
    const r = await llmApi.llmTestConnection(p.id)
    testResults.value = { ...testResults.value, [p.id]: r }
    if (r.ok) ElMessage.success(`连通正常（${r.latencyMs} ms）`)
    else ElMessage.warning(r.message || '连不通，请检查地址与密钥')
  } catch (e) {
    // 「配置问题」走 Err，「连不通」走 ok:false —— 两者要分开呈现
    const msg = errorMessage(e as never)
    testResults.value = {
      ...testResults.value,
      [p.id]: { ok: false, model: p.model, latencyMs: 0, message: msg },
    }
    ElMessage.error(msg)
  } finally {
    testingId.value = null
  }
}

// ---------------------------------------------------------------- 提示词

const promptA = computed(() => snapshot.value.strings[PREF_KEYS.promptA] ?? '')
const defaultReminder = computed(() => snapshot.value.strings[PREF_KEYS.defaultReminder] ?? '')
const generateExplanation = computed(
  () => (snapshot.value.ints[PREF_KEYS.generateExplanation] ?? 1) !== 0,
)

/** 提示词改动要整体提交（`config_save` 是替换） */
async function saveStrings(patch: Record<string, string>): Promise<void> {
  const next: ConfigSnapshot = {
    strings: { ...snapshot.value.strings, ...patch },
    ints: { ...snapshot.value.ints },
  }
  try {
    await systemApi.configSave(next)
    snapshot.value = next
    notifySaved('已保存')
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  }
}

async function saveInt(key: string, value: number): Promise<void> {
  const next: ConfigSnapshot = {
    strings: { ...snapshot.value.strings },
    ints: { ...snapshot.value.ints, [key]: value },
  }
  try {
    await systemApi.configSave(next)
    snapshot.value = next
    notifySaved('已保存')
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  }
}

// ---------------------------------------------------------------- 文本框防抖落盘

/**
 * 提示词 / 提醒词的改动**防抖 500ms 再落盘**。
 *
 * ## 为什么要防抖
 *
 * `PromptEditor` 的 textarea 是 `@update:model-value` 直连保存的 ——
 * 也就是**每敲一个字就把整份偏好写一次盘**。提示词动辄几百字，
 * 写完一篇要写几百次文件（还带一次 `rename`），既慢又费 SSD。
 *
 * 界面仍然立刻更新（先改 `snapshot`），只是落盘推迟到停手之后。
 */
let textSaveTimer: number | null = null
let pendingText: Record<string, string> = {}

function saveStringsDebounced(patch: Record<string, string>): void {
  // 界面立刻反映（`promptA` 是 snapshot 的 computed）
  snapshot.value = {
    strings: { ...snapshot.value.strings, ...patch },
    ints: { ...snapshot.value.ints },
  }
  pendingText = { ...pendingText, ...patch }

  if (textSaveTimer !== null) window.clearTimeout(textSaveTimer)
  textSaveTimer = window.setTimeout(() => {
    textSaveTimer = null
    void flushTextSave()
  }, 500)
}

/** 立刻把待落盘的文本改动写掉（离开页面 / 重置前调） */
async function flushTextSave(): Promise<void> {
  if (textSaveTimer !== null) {
    window.clearTimeout(textSaveTimer)
    textSaveTimer = null
  }
  const patch = pendingText
  pendingText = {}
  if (Object.keys(patch).length === 0) return
  await saveStrings(patch)
}

/**
 * 设置页被 `keep-alive` 缓存 —— 切走时 `onBeforeUnmount` 不会跑，
 * 所以要在 `onDeactivated` 把防抖窗口里的改动补上，
 * 否则用户打完字立刻切页，最后一次改动会丢。
 */
onDeactivated(() => {
  void flushTextSave()
})

/**
 * 恢复内置提示词：删键（不是在前端塞默认文案）。
 *
 * ## 为什么要二次确认（需求 2）
 *
 * 这一下会把**自定义 Prompt A 整段删掉**（几百字的手工内容），
 * 而且**不可撤销**。它就在文本框下面，误点的成本很高。
 */
async function resetPrompts(): Promise<void> {
  try {
    await ElMessageBox.confirm(
      '会删掉你自定义的 Prompt A 与提醒词，回到内置文案。自定义内容**无法找回**（建议先复制备份）。',
      '恢复内置提示词？',
      { type: 'warning', confirmButtonText: '恢复内置', cancelButtonText: '取消' },
    )
  } catch {
    return // 用户取消
  }

  // 防抖窗口里可能还有没落盘的改动 —— 先丢掉，否则它会把刚删掉的键又写回去
  if (textSaveTimer !== null) {
    window.clearTimeout(textSaveTimer)
    textSaveTimer = null
  }
  pendingText = {}

  try {
    await systemApi.configResetSection('PROMPTS')
    await loadPrefs()
    ElMessage.success('已恢复内置提示词')
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  }
}

// ---------------------------------------------------------------- 导出选项

async function onExportOptions(o: ExportOptions): Promise<void> {
  exportOptions.value = o
  try {
    await reportApi.exportOptionsSave(o)
    notifySaved('导出选项已保存')
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  }
}

// ---------------------------------------------------------------- 配置迁移

const includeKeys = ref(false)
const migrating = ref(false)

async function exportConfig(): Promise<void> {
  if (includeKeys.value) {
    try {
      await ElMessageBox.confirm(
        '导出的文件里会包含**明文 API Key**。任何拿到这个文件的人都能用你的额度。',
        '确认导出密钥',
        { type: 'warning', confirmButtonText: '仍要导出', cancelButtonText: '取消' },
      )
    } catch {
      return
    }
  }

  migrating.value = true
  try {
    const json = await llmApi.llmConfigExport(includeKeys.value)
    const path = await pickJsonSavePath(`civilcalc-llm-config${includeKeys.value ? '-with-keys' : ''}.json`)
    if (!path) return
    await saveTextToFile(json, path)
    ElMessage.success('已导出')
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  } finally {
    migrating.value = false
  }
}

async function importConfig(): Promise<void> {
  migrating.value = true
  try {
    const path = await pickJsonFile()
    if (!path) return
    const json = await readTextFile(path)
    const report = await llmApi.llmConfigImport(json)
    const bits: string[] = []
    bits.push(report.configApplied ? '配置已应用' : '配置未变更')
    if (report.apiKeysApplied > 0) bits.push(`导入 ${report.apiKeysApplied} 个密钥`)
    if (report.skippedProfiles.length > 0) {
      bits.push(`跳过 ${report.skippedProfiles.length} 个本机不存在的档位`)
    }
    ElMessage.success(bits.join('，'))
    await loadLlm()
  } catch (e) {
    ElMessage.error(errorMessage(e as never))
  } finally {
    migrating.value = false
  }
}

// ---------------------------------------------------------------- 重置

/**
 * 重置完成后把**所有派生状态**重新对齐（需求 9 / 11）。
 *
 * ## 为什么不能只重载设置页自己的数据
 *
 * 重置会动到外观、模型配置、提示词、草稿…… 这些东西散落在好几个 store 里。
 * 只重载本页的话，其它页面还拿着被删掉的值：
 *
 * - 侧栏 / 卡片还是旧主题、旧背景、旧文字色（外观在 `ui` store + DOM 上）
 * - 「新建公式」「模型测试」还显示着已被删掉的模型名（共享状态）
 * - 刚生成出来的思考暂存还挂着已被删掉的公式 id
 * - 公式工作台的「正在使用哪条公式」标记还指向不存在的公式
 *
 * 所以这里逐个清干净，而不是只 `loadLlm()`。
 */
async function onResetDone(): Promise<void> {
  // 重置会清掉提示词 —— 先把防抖窗口里待写的改动丢掉，否则它会把刚重置的键又写回去
  if (textSaveTimer !== null) {
    window.clearTimeout(textSaveTimer)
    textSaveTimer = null
  }
  pendingText = {}
  await Promise.all([loadLlm(), loadPrefs(), loadExportOptions(), ui.loadAppearance()])
  // 模型配置可能被清空 → 回到内置三家预设，共享状态要重拉
  resetActiveModel()
  await refreshActiveModel(true)
  // 被删掉的公式不该再被认为是「正在使用中」
  formulaStore.forgetWorkspace()
  // 会话暂存的思考也一并清掉
  thinkingStore.clear()
  ElMessage.info('重置完成，已重新加载设置')
}
</script>

<template>
  <div class="view">
    <!-- 品牌头（需求 8）：与「关于」页同一套标识，整站风格统一。
         它不参与 loading —— 骨架屏期间也显示，避免页面顶部空一块。 -->
    <header class="brand">
      <AppLogo :size="40" />
      <div class="brand__main">
        <h2 class="brand__title">设置</h2>
        <p class="brand__sub">模型与密钥、AI 行为、外观、导出与数据维护</p>
      </div>
    </header>

    <el-skeleton v-if="loading" :rows="8" animated />

    <template v-else>
      <!-- ① 模型与密钥 -->
      <section class="panel">
        <button
          class="panel__head panel__head--toggle"
          type="button"
          :aria-expanded="!isCollapsed('llm')"
          @click="toggleSection('llm')"
        >
          <span class="panel__caret" aria-hidden="true">{{ isCollapsed('llm') ? '▸' : '▾' }}</span>
          <span class="panel__title">模型与密钥</span>
          <span class="panel__hint">密钥只进系统凭据管理器，不随配置导出</span>
        </button>
        <div v-show="!isCollapsed('llm')" class="panel__body">
          <p v-if="config.profiles.length === 0" class="empty">
            还没有模型档位。添加一个并填入 API Key 后就能使用 AI 解析。
          </p>
          <ModelProfileList
            v-else
            :config="config"
            :key-status="keyStatus"
            :testing-id="testingId"
            :test-results="testResults"
            @set-active="onSetActive"
            @create="onCreate"
            @edit="onEdit"
            @remove="onRemove"
            @test="onTest"
          />
          <el-button v-if="config.profiles.length === 0" type="primary" @click="onCreate()">
            添加档位
          </el-button>
        </div>
      </section>

      <!-- ② AI 行为 -->
      <section class="panel">
        <button
          class="panel__head panel__head--toggle"
          type="button"
          :aria-expanded="!isCollapsed('ai')"
          @click="toggleSection('ai')"
        >
          <span class="panel__caret" aria-hidden="true">{{ isCollapsed('ai') ? '▸' : '▾' }}</span>
          <span class="panel__title">AI 行为</span>
          <span class="panel__hint">解析提示词 / 默认提醒词 / 是否生成详解</span>
        </button>
        <div v-show="!isCollapsed('ai')" class="panel__body">
          <PromptEditor
            :prompt-a="promptA"
            :default-reminder="defaultReminder"
            :generate-explanation="generateExplanation"
            @update:prompt-a="(v: string) => saveStringsDebounced({ [PREF_KEYS.promptA]: v })"
            @update:default-reminder="(v: string) => saveStringsDebounced({ [PREF_KEYS.defaultReminder]: v })"
            @update:generate-explanation="(v: boolean) => saveInt(PREF_KEYS.generateExplanation, v ? 1 : 0)"
            @reset="resetPrompts"
          />
        </div>
      </section>

      <!-- ③ 外观 -->
      <section class="panel">
        <button
          class="panel__head panel__head--toggle"
          type="button"
          :aria-expanded="!isCollapsed('appearance')"
          @click="toggleSection('appearance')"
        >
          <span class="panel__caret" aria-hidden="true">{{ isCollapsed('appearance') ? '▸' : '▾' }}</span>
          <span class="panel__title">外观</span>
        </button>
        <div v-show="!isCollapsed('appearance')" class="panel__body">
          <AppearancePanel />
        </div>
      </section>

      <!-- ④ 导出选项 -->
      <section class="panel">
        <button
          class="panel__head panel__head--toggle"
          type="button"
          :aria-expanded="!isCollapsed('export')"
          @click="toggleSection('export')"
        >
          <span class="panel__caret" aria-hidden="true">{{ isCollapsed('export') ? '▸' : '▾' }}</span>
          <span class="panel__title">计算书导出选项</span>
          <span class="panel__hint">导出时也会记住你在对话框里的临时选择</span>
        </button>
        <div v-show="!isCollapsed('export')" class="panel__body">
          <ExportOptionsForm :options="exportOptions" @update="onExportOptions" />
        </div>
      </section>

      <!-- ⑤ 配置迁移 -->
      <section class="panel">
        <button
          class="panel__head panel__head--toggle"
          type="button"
          :aria-expanded="!isCollapsed('migrate')"
          @click="toggleSection('migrate')"
        >
          <span class="panel__caret" aria-hidden="true">{{ isCollapsed('migrate') ? '▸' : '▾' }}</span>
          <span class="panel__title">配置迁移</span>
        </button>
        <div v-show="!isCollapsed('migrate')" class="panel__body">
          <p class="hint">
            导出/导入模型档位与密钥，用于换机或重装。导入时包里缺密钥就不动本机密钥。
          </p>
          <div class="migrate">
            <el-button :loading="migrating" @click="exportConfig()">导出配置</el-button>
            <el-button :loading="migrating" @click="importConfig()">导入配置</el-button>
            <el-checkbox v-model="includeKeys">导出时包含明文 API Key</el-checkbox>
          </div>
        </div>
      </section>

      <!-- ⑥ 重置 -->
      <section class="panel panel--danger">
        <button
          class="panel__head panel__head--toggle"
          type="button"
          :aria-expanded="!isCollapsed('reset')"
          @click="toggleSection('reset')"
        >
          <span class="panel__caret" aria-hidden="true">{{ isCollapsed('reset') ? '▸' : '▾' }}</span>
          <span class="panel__title">重置软件</span>
        </button>
        <div v-show="!isCollapsed('reset')" class="panel__body">
          <ResetPanel @done="onResetDone" />
        </div>
      </section>
    </template>

    <ModelProfileForm
      v-model="formOpen"
      :profile="editing"
      :creating="creating"
      :has-key="editingHasKey"
      @submit="onSubmit"
      @delete-key="onDeleteKey"
    />
  </div>
</template>

<style scoped>
.view {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  max-width: 1200px;
}

/* 品牌头：与「关于」页同一套排布（图标 + 标题 + 副标题） */
.brand {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
  padding: var(--sp-2) var(--sp-1);
}

.brand__main {
  min-width: 0;
}

.brand__title {
  margin: 0;
  color: var(--c-text);
  font-size: var(--f-size-xl);
  font-weight: 600;
}

.brand__sub {
  margin: 2px 0 0;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.panel {
  border-radius: var(--r-card);
  background: var(--c-surface);
}

.panel--danger {
  border-left: 3px solid var(--c-danger);
}

.panel__head {
  display: flex;
  /*
   * 🔴 上下内边距**必须对称**，而且 `align-items` 要用 `center`。
   *
   * 这里原本是 `padding: var(--sp-4) var(--sp-4) 0`（底部 0）+ `align-items: baseline`：
   * 那套是给「下面紧跟着正文」的静态标题设计的，底部不留白正好。
   * 但设置页这 6 个分块**全部可折叠** —— 收起之后标题下面什么都没有，
   * 那 0 的底部内边距就让文字**贴到了底边**，看起来像没对齐。
   *
   * 与 `FormulaView` 的折叠栏（「Excel 公式」那种）保持同一套：
   * 四边 `var(--sp-4)` + 垂直居中。
   */
  align-items: center;
  gap: var(--sp-3);
  padding: var(--sp-4);
  /*
   * ⚠️ 这里**不要**用 `justify-content: space-between`。
   *
   * 只有两个子项时它正好是「标题靠左、说明靠右」；但「AI 行为」那一栏是
   * **三个**子项（caret / 标题 / 说明），space-between 会把**中间的标题推到正中** ——
   * 看起来和别的折叠栏标题（都靠左）不对称。
   *
   * 靠右交给 `.panel__hint` 自己的 `margin-left: auto`，两种结构都对。
   */
}

/* 可折叠的区块标题：整行可点，且要重置按钮的默认外观 */
.panel__head--toggle {
  width: 100%;
  border: none;
  background: transparent;
  font-family: inherit;
  font-size: inherit;
  text-align: left;
  cursor: pointer;
}

.panel__head--toggle:hover .panel__title {
  color: var(--c-primary);
}

.panel__caret {
  flex-shrink: 0;
  width: 12px;
  color: var(--c-text-3);
}

/* 标题行右侧的补充说明：靠右由自己撑开（父级不用 space-between，见上） */
.panel__hint {
  margin-left: auto;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.panel__title {
  font-weight: 600;
  color: var(--c-text);
}

.panel__body {
  padding: var(--sp-3) var(--sp-4) var(--sp-4);
}

.empty {
  margin: 0 0 var(--sp-3);
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
}

.hint {
  margin: 0 0 var(--sp-3);
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
  line-height: 1.7;
}

.migrate {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
  flex-wrap: wrap;
}
</style>
