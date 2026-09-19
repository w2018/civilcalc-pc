<script setup lang="ts">
/**
 * PromptEditor —— 提示词与提醒词（AI 解析的 Prompt A、结果区提醒、是否生成详解）。
 *
 * 见 docs/05-项目开发方案.md §2.1.2「设置」与 docs/04 的偏好键表。
 *
 * ## 🔴 自定义提示词是**整段替换**，不是追加
 *
 * 这一条决定了「写法帮助」怎么写。后端（`civilcalc_llm::normalizer`）的组装是：
 *
 * ```text
 * messages = [ system(自定义提示词 或 内置 PROMPT_A),  user(用户输入的需求/公式文本) ]
 * ```
 *
 * 所以：
 *
 * - 自定义内容**整个替换**内置 system 提示词 —— 不是拼在后面
 * - 用户的需求文本是**另一条 user 消息**，**不会**被替换进提示词里
 *
 * ⚠️ 早先这里检查过 `{{需求}}` 占位符 —— 那是**错的**：
 * 后端没有任何占位符替换逻辑，写了 `{{需求}}` 也不会被替换，
 * 反而会让用户以为必须写它。已改成检查真正致命的东西：
 * **JSON 输出契约还在不在**。
 *
 * ## 「恢复内置」用**删键**而不是把默认文案写在前端
 *
 * `config_reset_section('PROMPTS')` 删掉偏好键，后端就回落内置常量。
 * 如果在前端硬编码一份默认文案，就变成**两处维护**，
 * 后端改了提示词而前端没改，用户点「恢复默认」会得到一份旧文案。
 */
import { computed, ref } from 'vue'

const props = defineProps<{
  /** AI 解析提示词（偏好键 `prompt_a`；空串 = 用内置） */
  promptA: string
  /** 结果区默认提醒词（偏好键 `default_reminder`） */
  defaultReminder: string
  /** 生成公式时是否一并输出详解（偏好键 `generate_explanation`） */
  generateExplanation: boolean
}>()

const emit = defineEmits<{
  (e: 'update:promptA', v: string): void
  (e: 'update:defaultReminder', v: string): void
  (e: 'update:generateExplanation', v: boolean): void
  /** 恢复内置（父组件调 `config_reset_section('PROMPTS')` 后重载） */
  (e: 'reset'): void
}>()

const helpOpen = ref(false)

const usingCustom = computed(() => props.promptA.trim().length > 0)

/**
 * 自定义提示词是否**看起来**还带着输出契约。
 *
 * 判据是「出现 JSON 且出现 schemaVersion」—— 两者都是契约里的硬字段。
 * 只做**提示**不做拦截：用户可能有意用别的措辞表达同样的约束。
 */
const contractLooksMissing = computed(() => {
  const p = props.promptA
  if (!usingCustom.value) return false
  return !(/json/i.test(p) && /schemaVersion/.test(p))
})

/** 契约缺失时的提醒文案 */
const contractWarning = computed(() =>
  contractLooksMissing.value
    ? '自定义提示词里似乎没有「只输出 JSON」与 schemaVersion 字段。模型若不按契约返回 JSON，解析会直接失败 —— 建议先复制内置提示词再改。'
    : '',
)
</script>

<template>
  <div class="prompt">
    <label class="field">
      <span class="field__label">
        AI 解析提示词（Prompt A）
        <button class="help" type="button" @click="helpOpen = true">写法帮助</button>
      </span>
      <el-input
        :model-value="promptA"
        type="textarea"
        :rows="8"
        resize="vertical"
        placeholder="留空即使用内置提示词（推荐）"
        @update:model-value="(v: string) => emit('update:promptA', v)"
      />
      <span class="field__hint">
        留空即用内置提示词。<strong>填了会整段替换内置内容</strong>（不是追加），
        改动影响所有后续的公式解析。已填 {{ promptA.length }} 字。
      </span>
    </label>

    <p v-if="contractWarning" class="warn">{{ contractWarning }}</p>

    <label class="field">
      <span class="field__label">结果区默认提醒词</span>
      <!--
        ⚠️ 加 `reminder-box`：全站的 `el-input` 都被全局样式改成「只有底线」
        （见 `styles/element-override.scss`）。但这一项**必须有完整边框** ——
        它紧跟在「留空即用内置提示词」的大文本框下面，只有一条底线时
        看起来像一段普通文字，用户根本认不出那是个输入框（需求 2）。
      -->
      <el-input
        class="reminder-box"
        :model-value="defaultReminder"
        placeholder="如：结果需经注册工程师复核"
        @update:model-value="(v: string) => emit('update:defaultReminder', v)"
      />
      <span class="field__hint">显示在计算结果下方，留空则不显示。</span>
    </label>

    <div class="checks">
      <el-checkbox
        :model-value="generateExplanation"
        @update:model-value="(v: boolean | string | number) =>
          emit('update:generateExplanation', !!v)"
      >
        解析公式时一并生成「计算公式详解」
      </el-checkbox>
      <span class="checks__hint">关掉后详解留空，可在工作台里按需单独生成（省 token）。</span>
    </div>

    <div class="prompt__foot">
      <el-button :disabled="!usingCustom" @click="emit('reset')">恢复内置提示词</el-button>
    </div>

    <!-- 写法帮助（需求 12） -->
    <el-dialog v-model="helpOpen" title="提示词写法帮助" width="680px">
      <div class="help-body">
        <section class="hb">
          <h4 class="hb__title">先理解它怎么被用</h4>
          <p class="hb__text">
            你的提示词会作为 <strong>system 消息</strong>整段发给模型；
            你输入的需求文本是<strong>另一条 user 消息</strong>。两者是分开的。
          </p>
          <p class="hb__text hb__text--warn">
            所以：<strong>填了就整段替换内置提示词</strong>（不是追加），
            而且<strong>不需要</strong>写占位符 —— 后端没有任何占位符替换逻辑，
            写了也不会生效。
          </p>
        </section>

        <section class="hb">
          <h4 class="hb__title">必须保留的三件事</h4>
          <ol class="hb__list">
            <li>
              <strong>只输出 JSON</strong>，禁止解释与 Markdown 包裹。
              少了这条模型会聊天式回答，解析直接失败。
            </li>
            <li>
              <strong>字段清单与顺序</strong>（含 <code>schemaVersion</code>）。
              少一个字段就少一块数据，前端拿不到就显示空。
            </li>
            <li>
              <strong>允许函数表</strong>（<code>sqrt / abs / pow / log / …</code>）。
              表达式只能用表里的函数，否则求值会报错。
            </li>
          </ol>
          <p class="hb__text">
            最稳的做法：<strong>先复制内置提示词，只改你要改的部分</strong>
            （语气、领域偏好、额外约束），契约段落原样保留。
          </p>
        </section>

        <section class="hb">
          <h4 class="hb__title">详解正文的三档标记（仅在生成详解时用到）</h4>
          <table class="hb__table">
            <thead>
              <tr>
                <th>写法</th>
                <th>用途</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <td><code>**加粗**</code></td>
                <td>关键依据 / 结论 / 系数含义</td>
              </tr>
              <tr>
                <td><code>`行内代码`</code></td>
                <td>变量符号 / 系数数值 / 单位</td>
              </tr>
              <tr>
                <td><code>==高亮==</code></td>
                <td>每步最核心的结果（金色荧光底），每步 2~5 处，禁整句</td>
              </tr>
              <tr>
                <td><code>&#123;&#123;img:N&#125;&#125;</code></td>
                <td>
                  插在第 N 张附图该出现的位置（序号从 1 起，按附图顺序）。
                  禁止编造不存在的序号；没附图就不能出现
                </td>
              </tr>
            </tbody>
          </table>
        </section>

        <section class="hb">
          <h4 class="hb__title">可以怎么改（按风险从低到高）</h4>
          <ul class="hb__list">
            <li><strong>低风险</strong>：加一句领域偏好，如「优先采用中国现行规范的习惯写法」</li>
            <li><strong>低风险</strong>：约束命名风格，如「resultName 用『构件-受力状态-结果』格式」</li>
            <li><strong>中风险</strong>：调整 <code>designNotes</code> 的详略与语气</li>
            <li>
              <strong>高风险</strong>：改动字段清单或允许函数表 —— 会让前端解析不到数据，
              除非你同时改后端代码
            </li>
          </ul>
        </section>

        <p class="hb__foot">
          改坏了不要紧：点「恢复内置提示词」即可删掉自定义内容，回到内置版本。
        </p>
      </div>

      <template #footer>
        <el-button type="primary" @click="helpOpen = false">知道了</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
.prompt {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
}

.field {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
}

/* 「结果区默认提醒词」那个输入框曾经在这里单独覆盖成完整边框 ——
 * 现在全局 `el-input` 本身就是四边框了（见 `styles/element-override.scss`），
 * 这段重复样式已删除，避免两处各写一遍、以后改一处漏一处。 */

.field__label {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
  font-size: var(--f-size-sm);
  color: var(--c-text-2);
}

.field__hint {
  font-size: var(--f-size-xs);
  color: var(--c-text-3);
  line-height: 1.6;
}

.help {
  padding: 0 var(--sp-2);
  border: var(--hairline) solid var(--c-primary);
  border-radius: var(--r-pill);
  background: transparent;
  color: var(--c-primary);
  font-family: inherit;
  font-size: var(--f-size-xs);
  line-height: 18px;
  cursor: pointer;
}

.help:hover {
  background: var(--c-primary-light);
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

.checks {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
}

.checks__hint {
  padding-left: var(--sp-5);
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.prompt__foot {
  display: flex;
}

/* ---- 帮助弹窗 ---- */

.help-body {
  display: flex;
  flex-direction: column;
  gap: var(--sp-4);
  max-height: 60vh;
  overflow-y: auto;
}

.hb__title {
  margin: 0 0 var(--sp-2);
  font-size: var(--f-size-base);
  font-weight: 600;
  color: var(--c-text);
}

.hb__text {
  margin: 0 0 var(--sp-2);
  color: var(--c-text-2);
  font-size: var(--f-size-sm);
  line-height: 1.8;
}

.hb__text:last-child {
  margin-bottom: 0;
}

.hb__text--warn {
  padding: var(--sp-2) var(--sp-3);
  border-radius: var(--r-sm);
  background: var(--c-surface-2);
  color: var(--c-warning);
}

.hb__list {
  margin: 0;
  padding-left: var(--sp-5);
  color: var(--c-text-2);
  font-size: var(--f-size-sm);
  line-height: 1.9;
}

.hb__table {
  width: 100%;
  border-collapse: collapse;
  font-size: var(--f-size-sm);
}

.hb__table th,
.hb__table td {
  padding: var(--sp-2) var(--sp-3);
  border-bottom: var(--hairline) solid var(--c-divider);
  text-align: left;
  color: var(--c-text-2);
  vertical-align: top;
}

.hb__table th {
  background: var(--c-surface-2);
  color: var(--c-text-3);
  font-weight: 500;
}

.hb__foot {
  margin: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

code {
  padding: 0 4px;
  border-radius: 4px;
  background: var(--c-surface-2);
  font-family: var(--f-mono);
  font-size: var(--f-size-xs);
}
</style>
