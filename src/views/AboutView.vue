<script setup lang="ts">
/**
 * 关于 —— 版本、路径、开源许可、检查更新。
 *
 * 见 docs/05-项目开发方案.md §2.1.2「关于」与 `docs/07` 的 P5-3。
 *
 * ## 检查更新必须把「没更新」与「没查成」分开说
 *
 * `update_check` 不会抛错（失败不该弹错误框），但会如实回报三档状态：
 * 没查成（`checkOk = false` + `failReason`）/ 降级查到（`degraded`）/
 * 真查到了。这里按状态分别呈现 ——
 * **说「已是最新」而实际是断网或接口限流，用户会以为更新功能坏了。**
 *
 * 实测撞过：GitHub 匿名 API 是 60 次/小时/IP，限流时旧实现只会显示
 * 「没有可用的更新」。
 *
 * ## 桌面端不自下载
 *
 * PC 走 NSIS / MSI 安装包，`update_open_download` 只负责**用系统浏览器
 * 打开下载页**，剩下由用户手动完成（源项目的 APK 自下载不适用于桌面）。
 *
 * ## 路径信息用于排障
 *
 * 用户报「数据丢了」时，第一件事就是确认数据库到底在哪儿。
 * 所以这里把 DB / 数据 / 缓存 / 日志四个路径都列出来，并给「打开」按钮
 * （用系统文件管理器定位，而不是让用户手抄路径）。
 */
import { onMounted, ref } from 'vue'
import { reportApi, systemApi, updateApi } from '@/api'
import { normalizeError } from '@/api/invoke'
import AppLogo from '@/components/common/AppLogo.vue'
import { errorMessage } from '@/types/error'
import type { UpdateFailReason, UpdateInfo } from '@/api/update'
import type { AppInfo } from '@/types/system'

const info = ref<AppInfo | null>(null)
const loading = ref(true)

const checking = ref(false)
/** 检查结果（`null` = 还没查过） */
const update = ref<UpdateInfo | null>(null)

onMounted(async () => {
  try {
    info.value = await systemApi.appInfo()
  } catch (e) {
    ElMessage.error(errorMessage(normalizeError(e)))
  } finally {
    loading.value = false
  }
})

async function checkUpdate(): Promise<void> {
  if (checking.value) return
  checking.value = true
  try {
    update.value = await updateApi.updateCheck()
    if (update.value.hasUpdate) {
      ElMessage.success(`发现新版本 ${update.value.latestVersion}`)
    }
  } catch (e) {
    // 契约上不该走到这里（网络失败是静默的），但兜一下
    ElMessage.error(errorMessage(normalizeError(e)))
  } finally {
    checking.value = false
  }
}

/**
 * 失败原因码 → 给用户看的一句话。
 *
 * 只说我们**确实知道**的事，不编。`rateLimited` 那条尤其要写清楚
 * 「稍后重试即可」—— 否则用户会以为是自己网络的问题。
 */
function failText(reason: UpdateFailReason | null): string {
  switch (reason) {
    case 'offline':
      return '连不上 GitHub（网络不通或请求超时）。请检查网络后重试。'
    case 'rateLimited':
      return 'GitHub 的匿名接口限流了（每个 IP 每小时 60 次）。这是暂时性的，稍后重试即可。'
    case 'notFound':
      return 'GitHub 上找不到这个仓库，或者它还没有发布过版本。'
    case 'httpError':
      return 'GitHub 接口返回了异常状态，可能是服务临时故障。'
    case 'parseError':
      return 'GitHub 返回的内容解析不了，接口可能改版了。'
    default:
      return '未知原因。'
  }
}

async function openDownload(): Promise<void> {
  const url = update.value?.downloadUrl
  if (!url) return
  try {
    await updateApi.updateOpenDownload(url)
  } catch (e) {
    ElMessage.error(errorMessage(normalizeError(e)))
  }
}

/** 在系统文件管理器里定位路径（文件不存在时后端报 notFound） */
async function reveal(path: string | undefined): Promise<void> {
  if (!path) return
  try {
    await reportApi.reportReveal(path)
  } catch (e) {
    ElMessage.error(errorMessage(normalizeError(e)))
  }
}

const paths = () =>
  info.value
    ? [
        { key: '数据库', value: info.value.dbPath },
        { key: '数据目录', value: info.value.appDataDir },
        { key: '缓存目录', value: info.value.appCacheDir },
        { key: '日志目录', value: info.value.logsDir },
      ]
    : []
</script>

<template>
  <div class="view">
    <!-- ① 版本 -->
    <section class="panel">
      <div class="panel__body head">
        <AppLogo class="head__icon" :size="56" />
        <div class="head__main">
          <h2 class="head__name">{{ info?.productName || 'AI全能计算器' }}</h2>
          <p class="head__ver">
            版本 {{ info?.version || '—' }}
            <span v-if="info" class="head__schema">（数据库 schema v{{ info.dbSchemaVersion }}）</span>
          </p>
        </div>
        <el-button :loading="checking" @click="checkUpdate()">检查更新</el-button>
      </div>

      <!-- 更新结果 -->
      <div v-if="update" class="upd">
        <!-- ① 没查成 —— 绝不能显示成「已是最新」 -->
        <template v-if="!update.checkOk">
          <p class="upd__title upd__title--warn">没能查到更新信息</p>
          <p class="upd__meta">{{ failText(update.failReason) }}</p>
          <div class="upd__ops">
            <el-button @click="checkUpdate()">重试</el-button>
          </div>
        </template>

        <!-- ② 查到有新版本 -->
        <template v-else-if="update.hasUpdate">
          <p class="upd__title">
            发现新版本 {{ update.latestVersion }}（当前 {{ update.currentVersion }}）
          </p>
          <p v-if="update.degraded" class="upd__meta">
            ⚠️ {{ failText(update.failReason) }}版本号是走**网页兜底**拿到的，
            没有安装包大小与校验值，下载后请自行核对。
          </p>
          <p v-else-if="update.assetSize > 0" class="upd__meta">
            安装包 {{ updateApi.humanSize(update.assetSize) }}
            <template v-if="update.assetDigest">
              · 校验 {{ update.assetDigest.slice(0, 19) }}…
            </template>
          </p>
          <pre v-if="update.releaseNotes" class="upd__notes">{{ update.releaseNotes }}</pre>
          <div class="upd__ops">
            <el-button type="primary" :disabled="!update.downloadUrl" @click="openDownload()">
              {{ update.degraded ? '打开发布页' : '打开下载页' }}
            </el-button>
            <span v-if="!update.downloadUrl" class="upd__warn">
              这个 release 没有可识别的安装包，请手动到发布页下载
            </span>
          </div>
        </template>

        <!-- ③ 真查到了、且确实是最新 -->
        <template v-else>
          <p class="upd__title">已是最新（v{{ update.currentVersion }}）</p>
        </template>
      </div>

      <!-- 作者信息 —— 放在「版本」卡片最底部（更新结果之后） -->
      <div class="author">
        <span class="kv__key">作者</span>
        <span class="kv__val author__name">曾先生</span>
      </div>
    </section>

    <!-- ② 运行信息 -->
    <section class="panel">
      <div class="panel__head">
        <span class="panel__title">运行信息</span>
      </div>
      <div class="panel__body">
        <el-skeleton v-if="loading" :rows="3" animated />
        <template v-else-if="info">
          <div class="kv">
            <div class="kv__row">
              <span class="kv__key">内置公式</span>
              <span class="kv__val">{{ info.builtinFormulaCount }} 条</span>
            </div>
            <div class="kv__row">
              <span class="kv__key">库中公式</span>
              <span class="kv__val">{{ info.formulaCount }} 条（内置 + 自建）</span>
            </div>
            <div class="kv__row">
              <span class="kv__key">应用标识</span>
              <span class="kv__val kv__val--mono">{{ info.identifier }}</span>
            </div>
          </div>

          <div class="paths">
            <div v-for="p in paths()" :key="p.key" class="path">
              <span class="path__key">{{ p.key }}</span>
              <span class="path__val">{{ p.value }}</span>
              <button class="link" type="button" @click="reveal(p.value)">打开</button>
            </div>
          </div>

          <p class="hint">
            数据都在本机。换机时用「备份」页导出，或直接复制数据目录（数据库 +
            图片缓存 + 背景图都在里面）。
          </p>
        </template>
      </div>
    </section>

    <!-- ③ 许可 -->
    <section class="panel">
      <div class="panel__head">
        <span class="panel__title">开源许可</span>
      </div>
      <div class="panel__body">
        <p class="lic">
          本程序以 <strong>GNU General Public License v3.0（GPL-3.0）</strong> 发布，
          与源项目（Android 版）保持一致。你可以自由使用、修改与再分发，
          但**衍生作品也必须以同一许可开源**。
        </p>
        <p class="lic">
          内置公式与规范条文引用仅供工程计算参考。**计算结果需经注册工程师复核**
          —— 这一点也写在导出的计算书里。
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
  max-width: 880px;
}

.panel {
  border-radius: var(--r-card);
  background: var(--c-surface);
}

.panel__head {
  padding: var(--sp-4) var(--sp-4) 0;
}

.panel__title {
  font-weight: 600;
  color: var(--c-text);
}

.panel__body {
  padding: var(--sp-3) var(--sp-4) var(--sp-4);
}

/* 版本头 */
.head {
  display: flex;
  align-items: center;
  gap: var(--sp-4);
  padding: var(--sp-5) var(--sp-4);
}

/* 尺寸由 `size` 属性给（内联 SVG）；图形自带圆角，这里不再裁一次 */
.head__icon {
  flex-shrink: 0;
}

.head__main {
  flex: 1;
  min-width: 0;
}

/* 作者行：复用 `.kv__key` / `.kv__val` 的排版，只补一条与卡片内边距对齐的间距 */
.author {
  display: flex;
  gap: var(--sp-2);
  min-width: 0;
  padding: 0 var(--sp-4) var(--sp-4);
}

.author__name {
  font-weight: 600;
}

.head__name {
  margin: 0;
  color: var(--c-text);
  font-size: var(--f-size-xl);
  font-weight: 600;
}

.head__ver {
  margin: var(--sp-1) 0 0;
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
}

.head__schema {
  color: var(--c-text-3);
}

/* 更新结果 */
.upd {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  margin: 0 var(--sp-4) var(--sp-4);
  padding: var(--sp-3);
  border-radius: var(--r-card);
  background: var(--c-surface-2);
}

.upd__title {
  margin: 0;
  color: var(--c-text);
  font-size: var(--f-size-sm);
  font-weight: 500;
}

/* 「没能查到更新信息」—— 是失败，不是「已是最新」，用告警色区分开 */
.upd__title--warn {
  color: var(--c-warning);
}

.upd__meta {
  margin: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
  line-height: 1.7;
}

.upd__notes {
  margin: 0;
  max-height: 180px;
  overflow-y: auto;
  padding: var(--sp-2);
  border-radius: var(--r-sm);
  background: var(--c-surface);
  color: var(--c-text-2);
  font-family: var(--f-sans);
  font-size: var(--f-size-xs);
  line-height: 1.7;
  white-space: pre-wrap;
}

.upd__ops {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
  flex-wrap: wrap;
}

.upd__warn {
  color: var(--c-warning);
  font-size: var(--f-size-xs);
}

/* 键值 */
.kv {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--sp-2) var(--sp-4);
}

.kv__row {
  display: flex;
  gap: var(--sp-2);
  min-width: 0;
}

.kv__key {
  flex-shrink: 0;
  width: 72px;
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
}

.kv__val {
  color: var(--c-text);
  font-size: var(--f-size-sm);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.kv__val--mono {
  font-family: var(--f-mono);
  font-size: var(--f-size-xs);
}

/* 路径 */
.paths {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
  margin-top: var(--sp-3);
  padding-top: var(--sp-3);
  border-top: var(--hairline) solid var(--c-divider);
}

.path {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  min-width: 0;
}

.path__key {
  flex-shrink: 0;
  width: 72px;
  color: var(--c-text-3);
  font-size: var(--f-size-sm);
}

.path__val {
  flex: 1;
  min-width: 0;
  color: var(--c-text-2);
  font-family: var(--f-mono);
  font-size: var(--f-size-xs);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.hint {
  margin: var(--sp-3) 0 0;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
  line-height: 1.7;
}

.lic {
  margin: 0 0 var(--sp-2);
  color: var(--c-text-2);
  font-size: var(--f-size-sm);
  line-height: 1.8;
}

.lic:last-child {
  margin-bottom: 0;
}

.link {
  flex-shrink: 0;
  border: none;
  background: transparent;
  padding: 0;
  color: var(--c-primary);
  font-family: inherit;
  font-size: var(--f-size-sm);
  cursor: pointer;
}

.link:hover {
  text-decoration: underline;
}
</style>
