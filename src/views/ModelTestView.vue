<script setup lang="ts">
/**
 * 模型测试 —— 对话式测模型（多轮记忆 + 上下文压缩 + 实时统计）。
 *
 * 见 docs/05-项目开发方案.md §2.1.2「模型测试」与 §1.3.4 事件表。
 *
 * ## 为什么要有这一页
 *
 * 「设置 → 测试连接」只证明**连得上**，不证明**答得好**。
 * 这一页用一个真实的多轮对话来验：上下文能否保持、思考过程是否正常、
 * 用量是否合理。所以它和设置页的「测试连接」是两件事，别合并。
 *
 * ## 🔴 对话以**后端为准**，每轮结束后重新读回
 *
 * `model_test_send` 成功后才把「历史 + 本轮用户 + 助手回复」整体落盘，
 * 并且可能在中途**自动压缩**了历史。前端自己拼的列表会与磁盘不一致
 * （压缩后轮数会变少）。所以每轮结束都 `modelTestConversation()` 重读 ——
 * 界面显示的永远是「后端真正记住的东西」。
 *
 * ## 🔴 事件先订阅再发命令
 *
 * 载荷是增量（`+=`），命令 Promise 要等整轮结束才 resolve。
 *
 * ## 上下文占用是**估算**
 *
 * 用「字符数 / 2」（与后端同一口径）算占用，超过「上下文大小 × 阈值%」
 * 时后端会自动压缩。这里的进度条只是让用户**提前看到**这件事要发生了 ——
 * 精确 token 数只有厂商知道，不要假装精确。
 *
 * ## 系统提示词是这一页专属的
 *
 * 键 `model_test_system_prompt`，与公式解析用的 `prompt_a` **互不影响**。
 */
import { computed, nextTick, onActivated, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { aiApi, modelTestApi, systemApi } from '@/api'
import { normalizeError } from '@/api/invoke'
import ThinkingBox from '@/components/llm/ThinkingBox.vue'
import MarkdownText from '@/components/llm/MarkdownText.vue'
import ImageAttachRow from '@/components/image/ImageAttachRow.vue'
import InputHistoryButton from '@/components/common/InputHistoryButton.vue'
import { useInputHistory, type InputHistoryItem } from '@/composables/useInputHistory'
import { useActiveModel } from '@/composables/useActiveModel'
import { errorMessage, isCancelled } from '@/types/error'
import { PREF_KEYS, DEFAULT_TEST_CONTEXT_SIZE, DEFAULT_TEST_COMPRESS_THRESHOLD } from '@/types/system'
import { formatTokens } from '@/utils/format'
import { updatePrefsStrict } from '@/utils/prefsWrite'
import { notifySaved } from '@/utils/toast'
import {
  DEFAULT_TEST_SYSTEM_PROMPT,
  MAX_CONV_TURNS,
  conversationTokens,
  estimateTokens,
  type ModelTestTurn,
  type ModelTestUsagePayload,
} from '@/types/modelTest'

// ---------------------------------------------------------------- 状态

/** 已落盘的对话（以后端为准） */
const turns = ref<ModelTestTurn[]>([])
/** 本轮正在输入的内容 */
const draft = ref('')
/** 本轮附图 id */
const imageIds = ref<string[]>([])

/** 输入历史（需求 5）：点图标可把**文本 + 附图**一起恢复到输入框 */
const history = useInputHistory(PREF_KEYS.modelTestHistory)
void history.load()

function restoreFromHistory(it: InputHistoryItem): void {
  draft.value = it.text
  imageIds.value = [...it.imageIds]
  ElMessage.success('已恢复这条输入')
}

const sending = ref(false)
const loading = ref(true)

/** 本轮流式中的助手正文（**增量累加**） */
const streamingText = ref('')
/** 本轮思考过程（**增量累加**） */
const thinking = ref('')
const startedAt = ref<number | null>(null)

/** 本轮用量（后端只在 `modelTest://usage` 里给） */
const lastUsage = ref<ModelTestUsagePayload | null>(null)

/**
 * 读本轮用量。
 *
 * ⚠️ 为什么要包一层函数：`send()` 里刚写过 `lastUsage.value = null`，
 * TS 的控制流分析会把 `lastUsage.value` 收窄成 `null`，
 * 后面再判 `if (lastUsage.value && ...)` 就变成 `never`（编译报错）。
 * 经函数返回值读一次就没有这个收窄。
 */
function currentUsage(): ModelTestUsagePayload | null {
  return lastUsage.value
}
/** 本次会话累计用量（前端自己加；后端只记到全局用量表） */
const sessionUsage = ref({ prompt: 0, completion: 0, total: 0, cached: 0, calls: 0 })

/** 上下文配置（偏好） */
const contextSize = ref(DEFAULT_TEST_CONTEXT_SIZE)
const compressThreshold = ref(DEFAULT_TEST_COMPRESS_THRESHOLD)
const compressNotice = ref(true)

/** 系统提示词（本页专属） */
const systemPrompt = ref('')
const promptSaving = ref(false)
const promptOpen = ref(false)

/** 当前活跃档位名（共享状态，见 useActiveModel） */
const { activeLabel, refreshActiveModel } = useActiveModel()

const compressing = ref(false)

// ---------------------------------------------------------------- 初始化

let unlisten: (() => void) | null = null
const conversationRef = ref<HTMLElement | null>(null)

onMounted(async () => {
  // 先订阅：事件可能在命令返回前就到
  unlisten = await modelTestApi.subscribeModelTest({
    onThinking: (t) => {
      thinking.value += t
    },
    onContent: (t) => {
      streamingText.value += t
    },
    onUsage: (u) => {
      lastUsage.value = u
      sessionUsage.value = {
        prompt: sessionUsage.value.prompt + u.prompt,
        completion: sessionUsage.value.completion + u.completion,
        total: sessionUsage.value.total + u.total,
        cached: sessionUsage.value.cached + u.cached,
        calls: sessionUsage.value.calls + 1,
      }
    },
    onDone: () => {
      sending.value = false
    },
    onError: (e) => {
      sending.value = false
      if (!isCancelled(e)) ElMessage.error(errorMessage(e))
    },
  })

  await Promise.all([loadConversation(), loadPrefs(), loadModel()])
  loading.value = false
})

onBeforeUnmount(() => {
  unlisten?.()
  unlisten = null
})

/**
 * 本页被 `keep-alive` 缓存，`onMounted` 只跑一次。
 *
 * 在设置页改了活跃模型后切回来，这里必须重新读一次档位 ——
 * 否则标题栏还显示着上一个模型（需求 7）。
 * 设置页也会主动推送，这里是双保险。
 */
onActivated(() => {
  void refreshActiveModel(true)
})

async function loadConversation(): Promise<void> {
  try {
    turns.value = await modelTestApi.modelTestConversation()
  } catch (e) {
    ElMessage.error(errorMessage(normalizeError(e)))
  }
}

async function loadPrefs(): Promise<void> {
  try {
    const snap = await systemApi.configGet()
    systemPrompt.value = snap.strings[PREF_KEYS.testSystemPrompt] ?? ''
    contextSize.value = snap.ints[PREF_KEYS.testContextSize] ?? DEFAULT_TEST_CONTEXT_SIZE
    compressThreshold.value =
      snap.ints[PREF_KEYS.testCompressThreshold] ?? DEFAULT_TEST_COMPRESS_THRESHOLD
    compressNotice.value = (snap.ints[PREF_KEYS.testCompressNotice] ?? 1) !== 0
    // 上次没发出去的输入（崩溃后可恢复）
    draft.value = snap.strings[PREF_KEYS.modelTestInput] ?? ''
  } catch (e) {
    console.warn('[modelTest] 读偏好失败', e)
  }
}

async function loadModel(): Promise<void> {
  await refreshActiveModel(true)
}

/** 存一次当前输入（防止崩溃丢草稿） */
async function saveDraftInput(): Promise<void> {
  try {
    const snap = await systemApi.configGet()
    await systemApi.configSave({
      strings: { ...snap.strings, [PREF_KEYS.modelTestInput]: draft.value },
      ints: { ...snap.ints },
    })
  } catch {
    // 草稿存不上不是关键路径
  }
}

// ---------------------------------------------------------------- 统计

/** 估算的上下文占用（含流式中的本轮） */
const usedTokens = computed(
  () => conversationTokens(turns.value) + estimateTokens(draft.value) + estimateTokens(streamingText.value),
)

const limitTokens = computed(() => (contextSize.value * compressThreshold.value) / 100)

const usagePercent = computed(() => {
  if (limitTokens.value <= 0) return 0
  return Math.min(100, Math.round((usedTokens.value / limitTokens.value) * 100))
})

const nearLimit = computed(() => usagePercent.value >= 80)

/** 轮数是否接近上限（后端最多留 200 轮） */
const nearTurnLimit = computed(() => turns.value.length >= MAX_CONV_TURNS * 0.9)

// ---------------------------------------------------------------- 发送

async function send(): Promise<void> {
  if (sending.value) return
  const text = draft.value.trim()
  if (!text && imageIds.value.length === 0) {
    ElMessage.warning('请输入内容或添加图片')
    return
  }

  // 本轮用户消息（后端要求 messages 含本轮，排在最后）
  const userTurn: ModelTestTurn = { role: 'user', content: text }
  // 🔴 必须把**完整的** `turns`（含每轮助手轮的 `thinking`）传回去：
  // 后端落盘时沿用这份对象，自己反推的话思考会在每轮之间被丢掉。
  const payload = [...turns.value, userTurn]

  sending.value = true
  streamingText.value = ''
  thinking.value = ''
  lastUsage.value = null
  startedAt.value = Date.now()
  followBottom.value = true

  // 本地先乐观显示用户消息与空的助手气泡
  turns.value = payload
  draft.value = ''
  const images = [...imageIds.value]
  imageIds.value = []

  await nextTick()
  scrollToBottom()

  try {
    await modelTestApi.modelTestSend(payload, images)
    // 发送成功才记（需求 5）。用 `text` / `images` 这两个发送前就固定下来的局部量 ——
    // 此时 `draft` 与 `imageIds` 已经被清空，读它们会记成空历史
    void history.push(text, images)
    // 以磁盘为准重读（可能被自动压缩过）
    await loadConversation()
    const u = currentUsage()
    if (u && u.completion === 0) {
      ElMessage.warning('模型没有返回正文内容')
    }
  } catch (e) {
    const err = normalizeError(e)
    if (isCancelled(err)) {
      ElMessage.info('已取消')
      // 取消时后端不会落盘本轮 —— 把乐观插入的用户消息撤掉，避免界面与磁盘不一致
      await loadConversation()
    } else {
      ElMessage.error(errorMessage(err))
      await loadConversation()
    }
  } finally {
    sending.value = false
    streamingText.value = ''
    // 流式气泡被移除后版面会变矮 —— 再落一次底，否则会停在半空
    void followToBottom()
    await saveDraftInput()
  }
}

async function cancel(): Promise<void> {
  try {
    await aiApi.aiCancel()
  } catch {
    // 取消失败不打扰
  }
}

function scrollToBottom(): void {
  const el = conversationRef.value
  if (el) el.scrollTop = el.scrollHeight
}

// ---------------------------------------------------------------- 输出时自动滚到底（需求 3 / 4）

/**
 * 是否跟随到底部。
 *
 * 用户手动往上翻时置 `false` —— 否则他正看中间某一段，
 * 每个增量都把他拽回底部，根本读不了。
 * 重新回到底部附近（或自己发下一条）就恢复跟随。
 */
const followBottom = ref(true)

function onConvScroll(): void {
  const el = conversationRef.value
  if (!el) return
  followBottom.value = el.scrollHeight - el.scrollTop - el.clientHeight < 40
}

/** 跟到底（`nextTick` 后才能拿到新的 `scrollHeight`） */
async function followToBottom(): Promise<void> {
  if (!followBottom.value) return
  await nextTick()
  scrollToBottom()
}

/** 流式过程中：正文或思考每有新增就跟着滚 */
watch([streamingText, thinking], () => {
  if (!sending.value) return
  void followToBottom()
})

/**
 * 一轮结束后：**最终正文来自 `turns`**（后端重读后的结果），
 * 而不是 `streamingText`（它被清空了）。
 *
 * 所以只监听流式变量会漏掉「最后那一下」—— 表现为生成完了却停在半空，
 * 用户还得自己往下拖。这里补上「对话被重读后也要落到底」。
 */
watch(turns, () => {
  void followToBottom()
})

// ---------------------------------------------------------------- 输入框按键（需求 3）

/**
 * `Enter` 发送、`Ctrl+Enter` 换行。
 *
 * ## 为什么不是「Enter 换行」（textarea 默认）
 *
 * 这一页是**对话**，用户十有八九是在说一句话然后发出去。
 * 让 Enter 换行等于每次发送都要去够鼠标（或记一个组合键）。
 * 与主流聊天输入框一致：Enter 发送，要换行时按 Ctrl（或 Shift）+ Enter。
 *
 * ## 🔴 中文输入法必须放行
 *
 * 拼音候选框上按 Enter 是「选中候选词」，不是「发送」。
 * 只看 `key` 不看 `isComposing` 的话，用户打「混凝土」打到一半就被发出去。
 * `keyCode === 229` 是部分 IME 在 `isComposing` 不生效时的兜底标志。
 */
function onInputKeydown(e: Event | KeyboardEvent): void {
  // `el-input` 把 `keydown` 的载荷标成 `Event | KeyboardEvent`（它内部可能透传
  // 原生事件），这里先窄化再判键
  if (!(e instanceof KeyboardEvent)) return
  if (e.key !== 'Enter') return
  // 输入法组合中：交给 IME 处理
  if (e.isComposing || e.keyCode === 229) return
  // Ctrl / Shift + Enter → 换行（不拦截，走 textarea 默认行为）
  if (e.ctrlKey || e.metaKey || e.shiftKey) return

  e.preventDefault()
  void send()
}

// ---------------------------------------------------------------- 上下文管理

async function compress(): Promise<void> {
  if (compressing.value || sending.value) return
  if (turns.value.length === 0) {
    ElMessage.info('对话还是空的，没什么可压缩的')
    return
  }
  try {
    await ElMessageBox.confirm(
      `把当前 ${turns.value.length} 条对话压缩成一条摘要，**替代**原有记忆。摘要会丢细节，但能腾出上下文空间。`,
      '确认压缩上下文',
      { type: 'warning', confirmButtonText: '压缩', cancelButtonText: '取消' },
    )
  } catch {
    return
  }

  compressing.value = true
  try {
    await modelTestApi.modelTestCompress()
    await loadConversation()
    ElMessage.success('已压缩')
  } catch (e) {
    const err = normalizeError(e)
    if (isCancelled(err)) return
    ElMessage.error(errorMessage(err))
  } finally {
    compressing.value = false
  }
}

async function clearConversation(): Promise<void> {
  try {
    await ElMessageBox.confirm(
      '清空模型测试的全部对话记忆。**只影响这一页**，不动公式与历史。',
      '确认清空对话',
      { type: 'warning', confirmButtonText: '清空', cancelButtonText: '取消' },
    )
  } catch {
    return
  }
  try {
    await modelTestApi.modelTestClear()
    turns.value = []
    sessionUsage.value = { prompt: 0, completion: 0, total: 0, cached: 0, calls: 0 }
    lastUsage.value = null
    ElMessage.success('已清空')
  } catch (e) {
    ElMessage.error(errorMessage(normalizeError(e)))
  }
}

// ---------------------------------------------------------------- 模型设置（上下文 + 提示词）

/**
 * 默认提示词的**单行摘要**。
 *
 * 默认值本身是多行的（`\n` 分隔），直接塞进 `placeholder` 会把输入框
 * 撑成好几行、把面板顶变形。这里换成 ` / ` 连接。
 */
const defaultPromptOneLine = computed(() =>
  DEFAULT_TEST_SYSTEM_PROMPT.replace(/\s*\n\s*/g, ' / '),
)

/**
 * 落盘上下文设置（需求 4：支持自定义上下文大小）。
 *
 * ## 为什么走 `updatePrefsStrict` 而不是自己 `configGet` + `configSave`
 *
 * `config_save` 是**整体覆盖**。自己「读-改-写」会与其它偏好写入
 * （输入历史、提示词防抖落盘…）**交错互相覆盖** ——
 * 表现是「刚设的上下文大小过一会儿自己变回去了」。
 * `utils/prefsWrite.ts` 的队列把所有写入串成一条，从根上避免这件事。
 *
 * 用 `Strict` 版（会把错误抛出来）而不是静默版：设置没存上必须让用户知道，
 * 不能只弹一个「已保存」然后重启发现又变回去了。
 */
async function saveContextPrefs(): Promise<void> {
  try {
    await updatePrefsStrict((s) => ({
      strings: { ...s.strings },
      ints: {
        ...s.ints,
        [PREF_KEYS.testContextSize]: contextSize.value,
        [PREF_KEYS.testCompressThreshold]: compressThreshold.value,
      },
    }))
    notifySaved('上下文设置已保存')
  } catch (e) {
    ElMessage.error(errorMessage(normalizeError(e)))
  }
}

/** 恢复默认上下文（200k / 80%） */
async function resetContextPrefs(): Promise<void> {
  contextSize.value = DEFAULT_TEST_CONTEXT_SIZE
  compressThreshold.value = DEFAULT_TEST_COMPRESS_THRESHOLD
  await saveContextPrefs()
}

async function saveSystemPrompt(): Promise<void> {
  if (promptSaving.value) return
  promptSaving.value = true
  try {
    // 空串 = 回落默认（后端如此约定）
    await modelTestApi.modelTestSetSystemPrompt(systemPrompt.value.trim())
    notifySaved(systemPrompt.value.trim() ? '已保存系统提示词' : '已恢复默认系统提示词')
  } catch (e) {
    ElMessage.error(errorMessage(normalizeError(e)))
  } finally {
    promptSaving.value = false
  }
}

function resetSystemPrompt(): void {
  systemPrompt.value = ''
  void saveSystemPrompt()
}

// ---------------------------------------------------------------- 展示

const displayTurns = computed(() => turns.value)

function roleLabel(role: string): string {
  return role === 'user' ? '我' : '模型'
}

function n(v: number): string {
  return v.toLocaleString('zh-CN')
}

/** token 数的紧凑写法（`12345` → `12.3k`）—— 需求 1 要求显示成 `xxxk` */
function k(v: number): string {
  return formatTokens(v)
}

/**
 * 流式中的 token 估算（思考文本长度 / 2，与后端同一口径）。
 *
 * 拿到厂商真实 usage（`modelTest://usage`）后由 `lastUsage` 接管，
 * 那时 `tokensLive` 会变成 `false`，文案里的「约」字消失。
 */
const liveTokens = computed(() => {
  const u = lastUsage.value
  if (u) return u.total
  return Math.ceil(thinking.value.length / 2)
})
</script>

<template>
  <div class="view">
    <!-- ① 顶部状态条 -->
    <section class="panel panel--fixed">
      <div class="panel__body bar">
        <div class="bar__left">
          <span class="bar__model">{{ activeLabel || '未配置模型' }}</span>
          <span class="bar__sep">·</span>
          <span class="bar__stat">{{ turns.length }} 条消息</span>
          <span class="bar__sep">·</span>
          <span class="bar__stat">
            本次会话 {{ k(sessionUsage.total) }} tokens（{{ sessionUsage.calls }} 次调用）
          </span>
        </div>

        <div class="bar__ops">
          <el-button size="small" :disabled="sending || compressing" @click="compress()">
            压缩上下文
          </el-button>
          <el-button size="small" :disabled="sending" @click="clearConversation()">清空对话</el-button>
          <el-button size="small" @click="promptOpen = !promptOpen">
            {{ promptOpen ? '收起设置' : '模型设置' }}
          </el-button>
        </div>
      </div>

      <!-- 上下文占用 -->
      <div class="panel__body ctx">
        <div class="ctx__head">
          <span class="ctx__label">上下文占用（估算）</span>
          <span class="ctx__num" :class="{ 'ctx__num--warn': nearLimit }">
            {{ n(usedTokens) }} / {{ n(limitTokens) }} tokens
          </span>
        </div>
        <div class="ctx__bar" role="progressbar" :aria-valuenow="usagePercent" aria-valuemin="0" aria-valuemax="100">
          <div class="ctx__fill" :class="{ 'ctx__fill--warn': nearLimit }" :style="{ width: `${usagePercent}%` }" />
        </div>
        <p class="ctx__hint">
          <template v-if="compressNotice">
            超过 {{ compressThreshold }}% 时**发送前会自动压缩**历史（用「字符数 / 2」估算，与后端同口径）。
          </template>
          <template v-else>
            自动压缩提示已关闭（后端仍会按 {{ compressThreshold }}% 的阈值自动压缩）。
          </template>
          <template v-if="nearTurnLimit">⚠️ 消息数接近上限（{{ MAX_CONV_TURNS }}），最早的内容会被丢弃。</template>
        </p>
      </div>

      <!-- 模型设置：上下文 + 系统提示词（默认收起，展开才占地方） -->
      <div v-show="promptOpen" class="panel__body prompt">
        <!-- 上下文设置（需求 4：默认 200k，且可自定义） -->
        <div class="ctxcfg">
          <label class="ctxcfg__field">
            <span class="ctxcfg__label">上下文容量</span>
            <el-input-number
              v-model="contextSize"
              class="ctxcfg__num"
              :min="1000"
              :max="10000000"
              :step="1000"
              :controls="false"
              @change="saveContextPrefs"
            />
            <span class="ctxcfg__unit">tokens</span>
          </label>

          <label class="ctxcfg__field">
            <span class="ctxcfg__label">自动压缩阈值</span>
            <el-input-number
              v-model="compressThreshold"
              class="ctxcfg__num"
              :min="1"
              :max="100"
              :step="5"
              :controls="false"
              @change="saveContextPrefs"
            />
            <span class="ctxcfg__unit">%</span>
          </label>

          <button class="link" type="button" @click="resetContextPrefs()">
            恢复默认（200k / 80%）
          </button>
        </div>
        <p class="ctxcfg__hint">
          占用超过「容量 × 阈值」时，**发送前会把历史压成一条摘要**。
          这里的进度条用「字符数 ÷ 2」估算，与后端同一口径 —— 不是厂商的精确 token 数。
        </p>

        <label class="prompt__field">
          <span class="prompt__label">模型测试专属系统提示词</span>
          <el-input
            v-model="systemPrompt"
            type="textarea"
            :rows="3"
            resize="vertical"
            :placeholder="`留空即用默认：${defaultPromptOneLine}`"
          />
        </label>
        <p class="prompt__hint">
          与「设置 → AI 行为」里的解析提示词（Prompt A）**互不影响** —— 那条只管公式解析。
        </p>
        <div class="prompt__ops">
          <el-button size="small" type="primary" :loading="promptSaving" @click="saveSystemPrompt()">
            保存
          </el-button>
          <el-button size="small" @click="resetSystemPrompt()">恢复默认</el-button>
        </div>
      </div>
    </section>

    <!-- ② 对话 -->
    <section class="panel panel--conv">
      <div class="panel__body">
        <el-skeleton v-if="loading" :rows="4" animated />

        <div v-else-if="displayTurns.length === 0 && !sending" class="empty">
          <p class="empty__title">开始一次测试对话</p>
          <p class="empty__text">
            对话会跨进程保留（关掉应用再打开还在）。可以问它工程问题、试它的推理能力，
            或贴一张图纸让它描述 —— 这里的结果**不会**影响公式库。
          </p>
        </div>

        <div v-else ref="conversationRef" class="conv" @scroll="onConvScroll">
          <div v-for="(t, i) in displayTurns" :key="i" class="msg" :class="`msg--${t.role}`">
            <span class="msg__role">{{ roleLabel(t.role) }}</span>
            <div class="msg__body">
              <!-- 需求 3：历史里的助手轮保留当时的思考过程。
                   非流式 → 由 ThinkingBox 自动折叠（需求 1），点开可看全文。 -->
              <ThinkingBox
                v-if="t.role === 'assistant' && t.thinking"
                :text="t.thinking"
                title="思考过程"
              />
              <MarkdownText v-if="t.role === 'assistant'" :text="t.content" compact />
              <p v-else class="msg__plain">{{ t.content }}</p>
            </div>
          </div>

          <!-- 本轮流式 -->
          <template v-if="sending">
            <div class="msg msg--assistant">
              <span class="msg__role">模型</span>
              <div class="msg__body">
                <ThinkingBox
                  :text="thinking"
                  :streaming="true"
                  :started-at="startedAt"
                  :tokens="liveTokens"
                  :tokens-live="!lastUsage"
                  title="思考中"
                />
                <p v-if="!streamingText && !thinking" class="msg__waiting">等待模型响应…</p>
                <MarkdownText v-else-if="streamingText" :text="streamingText" compact />
              </div>
            </div>
          </template>
        </div>

        <!-- 本轮用量（需求 1：用 `xxxk` 紧凑写法） -->
        <p v-if="lastUsage" class="usage">
          本轮：输入 {{ k(lastUsage.prompt) }} · 输出 {{ k(lastUsage.completion) }} · 合计
          {{ k(lastUsage.total) }}<template v-if="lastUsage.cached > 0">
            · 缓存命中 {{ k(lastUsage.cached) }}</template>
        </p>
      </div>
    </section>

    <!-- ③ 输入
         高度敏感：文本框与发送同行、附图压成一行、提示只在有图时出现。
         输入区是 `flex-shrink: 0` 的，每多一行就直接从对话区扣掉一行。 -->
    <section class="panel panel--fixed">
      <div class="panel__body input">
        <div class="input__row">
          <el-input
            v-model="draft"
            class="input__box"
            type="textarea"
            :rows="2"
            resize="none"
            :disabled="sending"
            placeholder="输入测试内容，Enter 发送，Ctrl + Enter 换行"
            @keydown="onInputKeydown"
          />
          <InputHistoryButton
            :items="history.items.value"
            :disabled="sending"
            title="模型测试 · 输入历史"
            hint="从历史里选一条恢复（文本 + 附图）"
            @restore="restoreFromHistory"
            @remove="(i: number) => history.removeAt(i)"
            @clear="history.clear()"
          />
          <el-button v-if="!sending" type="primary" class="input__send" @click="send()">
            发送
          </el-button>
          <el-button v-else class="input__send" @click="cancel()">取消</el-button>
        </div>

        <ImageAttachRow v-model="imageIds" :max="5" :disabled="sending" compact />

        <p v-if="imageIds.length > 0" class="input__hint">
          图片只贴在本轮消息上，不写进对话记忆（避免配置被撑爆）
        </p>
      </div>
    </section>
  </div>
</template>

<style scoped>
.view {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  max-width: none;
  /* 🔴 需求 8：改成「整屏三段式」——
     状态 / 对话 / 输入各自固定角色，对话区内部滚动。
     没有 `height: 100%` 时整页会跟着对话长度一起变长，
     输入框被推到很远的地方，用户每发一条都要先滚到底。 */
  height: 100%;
  min-height: 0;
}

.panel {
  border-radius: var(--r-card);
  background: var(--c-surface);
}

/* 对话区：吃掉剩余高度，内部滚动 */
.panel--conv {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.panel--conv > .panel__body {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

/* 状态区与输入区不参与拉伸，保证输入框始终贴在底部可见 */
.panel--fixed {
  flex-shrink: 0;
}

/* ---- 输入区（需求：原来太高，把对话区挤没了）---- */

.input {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
}

.input__row {
  display: flex;
  align-items: flex-end;
  gap: var(--sp-2);
}

.input__box {
  flex: 1;
  min-width: 0;
}

/* 与文本框底部对齐，视觉上是一整条输入栏 */
.input__send {
  flex-shrink: 0;
}

.input__hint {
  margin: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.panel__body {
  padding: var(--sp-3) var(--sp-4);
}

.bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--sp-3);
  flex-wrap: wrap;
  padding-bottom: 0;
}

.bar__left {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  flex-wrap: wrap;
}

.bar__model {
  padding: 1px var(--sp-2);
  border-radius: var(--r-pill);
  background: var(--c-primary-light);
  color: var(--c-primary);
  font-size: var(--f-size-xs);
}

.bar__stat,
.bar__sep {
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.bar__ops {
  display: flex;
  gap: var(--sp-2);
}

/* 上下文占用 */
.ctx {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
}

.ctx__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.ctx__label {
  color: var(--c-text-2);
  font-size: var(--f-size-xs);
}

.ctx__num {
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
  font-variant-numeric: tabular-nums;
}

.ctx__num--warn {
  color: var(--c-warning);
}

.ctx__bar {
  height: 6px;
  border-radius: var(--r-pill);
  background: var(--c-surface-2);
  overflow: hidden;
}

.ctx__fill {
  height: 100%;
  border-radius: var(--r-pill);
  background: var(--c-primary);
  transition: width 0.2s linear;
}

.ctx__fill--warn {
  background: var(--c-warning);
}

.ctx__hint {
  margin: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
  line-height: 1.7;
}

/* 系统提示词 / 模型设置面板 */
.prompt {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  border-top: var(--hairline) solid var(--c-divider);
}

/* ---- 上下文设置（需求 4）---- */

.ctxcfg {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--sp-2) var(--sp-4);
}

.ctxcfg__field {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
}

.ctxcfg__label {
  color: var(--c-text-2);
  font-size: var(--f-size-sm);
}

.ctxcfg__num {
  width: 116px;
}

.ctxcfg__unit {
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.ctxcfg__hint {
  margin: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
  line-height: 1.7;
}

/* 面板内的文字按钮（「恢复默认（200k / 80%）」） */
.link {
  padding: 0;
  border: none;
  background: transparent;
  color: var(--c-primary);
  font-family: inherit;
  font-size: var(--f-size-xs);
  cursor: pointer;
}

.link:hover {
  text-decoration: underline;
}

.prompt__field {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
}

.prompt__label {
  color: var(--c-text-2);
  font-size: var(--f-size-sm);
}

.prompt__hint {
  margin: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.prompt__ops {
  display: flex;
  gap: var(--sp-2);
}

/* 对话 */
.empty {
  display: flex;
  flex-direction: column;
  /* 在对话区里居中（对话区是 flex 容器，空态吃掉全部高度） */
  flex: 1;
  align-items: center;
  justify-content: center;
  gap: var(--sp-1);
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

.conv {
  display: flex;
  flex-direction: column;
  gap: var(--sp-4);
  /* 由外层 flex 决定高度（需求 8）——不再写死 `52vh`：
     写死后窗口一放大，对话区还是那么高，而输入框被顶到屏幕外 */
  flex: 1;
  min-height: 0;
  overflow-y: auto;
}

.msg {
  display: flex;
  gap: var(--sp-3);
  align-items: flex-start;
}

.msg__role {
  flex-shrink: 0;
  width: 40px;
  padding-top: 2px;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
  text-align: right;
}

.msg__body {
  flex: 1;
  min-width: 0;
  padding: var(--sp-2) var(--sp-3);
  border-radius: var(--r-card);
  background: var(--c-surface-2);
}

.msg--user .msg__body {
  background: var(--c-primary-light);
}

/* 用户气泡用纯文本（那是他自己打的字），字号与紧凑版 Markdown 对齐 */
.msg__plain {
  margin: 0;
  color: var(--c-text);
  font-size: var(--f-size-sm);
  line-height: 1.6;
  white-space: pre-wrap;
  word-break: break-word;
}

.msg__waiting {
  margin: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
}

.usage {
  margin: var(--sp-3) 0 0;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
  font-variant-numeric: tabular-nums;
}
</style>
