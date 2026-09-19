<script setup lang="ts">
/**
 * 首次运行的「用户协议」弹窗。
 *
 * ## 为什么不可关闭
 *
 * 协议弹窗必须给出**明确的二选一**：同意才继续，不同意就退出。
 * 所以关掉三条退路 —— 点遮罩关闭 / Esc 关闭 / 右上角 ×。
 * 否则用户随手一关，既没同意也没退出，协议等于白弹。
 *
 * ## 同意状态存在哪
 *
 * 后端偏好（键 `eula_accepted`，见 `commands/system.rs`）。
 * ⚠️ 它**刻意不属于任何区块**，所以「重置软件」不会把它清掉 ——
 * 清了会让人下次启动又被弹一次，看起来像 bug。
 *
 * ## 读不到状态时**不弹**
 *
 * 协议弹窗是「拦人」的组件，不该因为一次读盘失败就把用户挡在门外。
 * 读失败只记日志、直接放行。
 *
 * ## ⚠️ 协议正文是**待确认的草稿**
 *
 * 下面 `SECTIONS` 里的条文是按本应用的实际行为写的（数据只存本机、
 * 只把内容发往用户自己配置的模型服务商、GPL-3.0、工程结果需自行复核），
 * **不是法务文本**。正式对外发布前请让用户确认或替换。
 */
import { onMounted, ref } from 'vue'
import { systemApi } from '@/api'
import { errorMessage } from '@/types/error'

/** 协议正文（草稿，见文件头说明） */
const SECTIONS: { title: string; lines: string[] }[] = [
  {
    title: '一、许可',
    lines: [
      '本软件以 GNU GPL-3.0 许可发布，按「原样」提供。你可以在该许可范围内自由使用、修改和分发。',
    ],
  },
  {
    title: '二、你的数据',
    lines: [
      '公式、历史记录、图片、配置等数据全部保存在本机，本软件不向任何自有服务器上传。',
      '模型 API Key 存放在 Windows 凭据管理器中，不写入配置文件。',
      '本软件没有遥测、没有埋点、不做任何后台数据上报。',
    ],
  },
  {
    title: '三、AI 功能',
    lines: [
      'AI 解析、续写、详解等功能会把相关内容发送到**你自己配置的**模型服务商。',
      '该部分数据的处理方式取决于你所选服务商的条款，请自行确认后再使用。',
    ],
  },
  {
    title: '四、计算结果必须自行复核',
    lines: [
      '本软件是计算辅助工具。内置公式的取值、规范条文的适用条件、以及 AI 生成的公式，',
      '都可能存在错误或与你所在项目不符的情况。',
      '**任何计算结果在用于实际工程设计、施工或结算前，必须由具备相应资格的人员独立复核。**',
      '因直接使用本软件结果而产生的任何后果，由使用者自行承担。',
    ],
  },
  {
    title: '五、免责',
    lines: [
      '本软件不提供任何明示或默示的担保，包括但不限于适销性、特定用途适用性与无侵权的担保。',
      '在法律允许的最大范围内，作者不对因使用或无法使用本软件而产生的任何损失承担责任。',
    ],
  },
]

const open = ref(false)
const saving = ref(false)
const error = ref('')

onMounted(async () => {
  try {
    open.value = !(await systemApi.eulaStatus())
  } catch (e) {
    // 读不到状态就不拦人（见文件头说明）
    console.warn('[EulaDialog] 读协议状态失败，跳过弹窗', e)
  }
})

async function accept(): Promise<void> {
  if (saving.value) return
  saving.value = true
  error.value = ''
  try {
    await systemApi.eulaAccept()
    open.value = false
  } catch (e) {
    error.value = errorMessage(e as never)
  } finally {
    saving.value = false
  }
}

/**
 * 不同意 → 退出应用。
 *
 * 后端这个命令**不返回**（延迟一小会儿直接结束进程），
 * 所以这里刻意不 `await` —— 等了也等不到，反而让按钮一直转圈。
 */
function decline(): void {
  void systemApi.appExit()
}
</script>

<template>
  <el-dialog
    v-model="open"
    title="用户协议"
    width="min(92vw, 720px)"
    align-center
    append-to-body
    :close-on-click-modal="false"
    :close-on-press-escape="false"
    :show-close="false"
    class="eula"
  >
    <p class="eula__lead">首次使用前请阅读并同意以下条款。</p>

    <div class="eula__body">
      <section v-for="s in SECTIONS" :key="s.title" class="eula__sec">
        <h3 class="eula__sec-title">{{ s.title }}</h3>
        <p v-for="(l, i) in s.lines" :key="i" class="eula__line">{{ l }}</p>
      </section>
    </div>

    <p v-if="error" class="eula__err">{{ error }}</p>

    <template #footer>
      <div class="eula__ops">
        <el-button @click="decline">不同意并退出</el-button>
        <el-button type="primary" :loading="saving" @click="accept">同意并继续</el-button>
      </div>
    </template>
  </el-dialog>
</template>

<style scoped>
.eula__lead {
  margin: 0 0 var(--sp-3);
  color: var(--c-text-2);
  font-size: var(--f-size-sm);
}

.eula__body {
  /* 条文比弹窗高，给个上限自己滚 */
  max-height: 52vh;
  overflow: auto;
  padding-right: var(--sp-2);
}

.eula__sec + .eula__sec {
  margin-top: var(--sp-4);
}

.eula__sec-title {
  margin: 0 0 var(--sp-1);
  color: var(--c-text);
  font-size: var(--f-size-sm);
  font-weight: 600;
}

.eula__line {
  margin: 0 0 var(--sp-1);
  color: var(--c-text-2);
  font-size: var(--f-size-sm);
  line-height: 1.8;
}

.eula__err {
  margin: var(--sp-3) 0 0;
  color: var(--c-danger, #e54545);
  font-size: var(--f-size-sm);
}

.eula__ops {
  display: flex;
  justify-content: flex-end;
  gap: var(--sp-3);
}
</style>
