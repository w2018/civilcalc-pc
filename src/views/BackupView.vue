<script setup lang="ts">
/**
 * 备份与恢复 —— 本地备份包 + WebDAV 云端备份。
 *
 * 见 docs/05-项目开发方案.md「流程 5：备份与恢复」与 §2.1.2。
 *
 * ## 两条链路，一个页面
 *
 * | 链路 | 命令 | 特点 |
 * |---|---|---|
 * | 本地 | `backup_local_export` / `backup_inspect` / `backup_local_import` | 落盘到「下载」，可离线 |
 * | 云端 | `webdav_*` 11 个 | 有进度、可取消、两段式（inspect → import） |
 *
 * 两边的**备份范围**与**加密密码**语义完全一样（同一个 `BackupSelection`），
 * 所以勾选表单与密码输入复用同一套组件/状态。
 *
 * ## 🔴 两个 `password` 同名不同物（本页同时出现）
 *
 * - **WebDAV 配置里的密码** = 连服务器的账号密码 → 存 keyring
 * - **备份包密码**（本页的「加密密码」）= 打开包的密码 → **不存**
 *
 * 传错不报错，只表现为「连不上」或「打不开包」。所以界面上把它们
 * 放在**不同的卡片**里，并且密码框的 placeholder 明确写清用途。
 *
 * ## 🔴 两段式：先 inspect 再 import
 *
 * 确认弹窗里的数字**来自包本身**（`inspect` 解析出的清单），不是估算。
 * 加密包没密码时 `inspect` 返回 `decoded: false`（**不是错误**）→
 * 就地让用户填密码重试。云端这条尤其重要：`webdav_inspect` 已把包
 * 下载到临时目录，重试**不会重新下载**（几十 MB 不传两遍）。
 *
 * ## 密码不落任何地方
 *
 * 只活在内存里，用完即清。页面不做「记住密码」—— 备份包密码忘了
 * 就是打不开，这是加密的应有之义，界面上要提前说明。
 */
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { backupApi, reportApi, systemApi, webdavApi } from '@/api'
import { onTauriEvent } from '@/api/events'
import { pickBackupFile } from '@/api/dialog'
import BackupSelectionForm from '@/components/backup/BackupSelectionForm.vue'
import ImportConfirmDialog from '@/components/backup/ImportConfirmDialog.vue'
import ProgressPanel from '@/components/backup/ProgressPanel.vue'
import WebDavConfigForm from '@/components/webdav/WebDavConfigForm.vue'
import RemoteFileList from '@/components/webdav/RemoteFileList.vue'
import { normalizeError } from '@/api/invoke'
import { errorMessage, isCancelled } from '@/types/error'
import { PREF_KEYS } from '@/types/system'
import {
  BACKUP_CANCELLED_EVENT,
  BACKUP_PROGRESS_EVENT,
  defaultBackupSelection,
  defaultWebDavConfig,
  describeSummary,
  humanSize,
  type BackupInspectResult,
  type BackupProgress,
  type BackupSelection,
  type ImportMode,
  type LocalExportResult,
  type RemoteBackup,
  type WebDavConfig,
} from '@/types/backup'

// ---------------------------------------------------------------- 状态

/** 备份范围（本地与云端共用） */
const selection = ref<BackupSelection>(defaultBackupSelection())

/** 本地导出 / 云端上传各自的加密密码（**不落任何地方**） */
const localPassword = ref('')
const cloudPassword = ref('')

const exporting = ref(false)
const uploading = ref(false)
const importing = ref(false)
/** 当前是否有长任务在跑（禁用所有操作） */
const busy = computed(() => exporting.value || uploading.value || importing.value)

const progress = ref<BackupProgress | null>(null)
const lastExport = ref<LocalExportResult | null>(null)

// WebDAV
const webdavConfig = ref<WebDavConfig>(defaultWebDavConfig())
const hasWebdavPassword = ref(false)
const remote = ref<RemoteBackup[]>([])
const remoteTotal = ref(0)
const remoteLoading = ref(false)
const lastBackupAt = ref<number | null>(null)
const backupCount = ref(0)

// 导入确认
const importOpen = ref(false)
const importInspect = ref<BackupInspectResult | null>(null)
const importSource = ref<'local' | 'cloud'>('local')
/** 导入目标：本地包用路径，云端包用文件名 */
const importTarget = ref<{ kind: 'local'; path: string } | { kind: 'cloud'; fileName: string } | null>(
  null,
)
/**
 * 这次 inspect 是**用哪个密码**解开的。
 *
 * 🔴 必须由页面持有：确认弹窗里重试成功后密码输入框会被清空，
 * 若那时再从弹窗取密码，导入就会「忘了密码」。
 */
const unlockedPassword = ref<string | null>(null)

// ---------------------------------------------------------------- 初始化

let unlisten: Array<() => void> = []

onMounted(async () => {
  // 先订阅再发命令（进度事件可能在命令返回前就来了）
  unlisten = await Promise.all([
    onTauriEvent<BackupProgress>(BACKUP_PROGRESS_EVENT, (p) => {
      progress.value = p
    }),
    onTauriEvent<Record<string, never>>(BACKUP_CANCELLED_EVENT, () => {
      // 取消的收尾在这里做：命令那边只会以 cancelled 拒绝，不再重复提示
      progress.value = null
      uploading.value = false
      importing.value = false
      ElMessage.info('已取消')
    }),
  ])

  await Promise.all([loadWebDav(), loadRemote(), loadStats()])
})

onBeforeUnmount(() => {
  for (const un of unlisten) un()
  unlisten = []
})

async function loadWebDav(): Promise<void> {
  try {
    webdavConfig.value = await webdavApi.webdavConfigGet()
    hasWebdavPassword.value = await webdavApi.webdavHasPassword()
  } catch (e) {
    console.warn('[backup] 读 WebDAV 配置失败', e)
  }
}

async function loadRemote(): Promise<void> {
  remoteLoading.value = true
  try {
    const r = await webdavApi.webdavList()
    remote.value = r.backups
    remoteTotal.value = r.total
  } catch (e) {
    // 没配好 WebDAV 时这里会失败 —— 不要弹红字（用户可能只是还没配）
    remote.value = []
    remoteTotal.value = 0
    console.warn('[backup] 读远端列表失败', e)
  } finally {
    remoteLoading.value = false
  }
}

/** 上次备份时间 / 累计次数（**不进备份包**的本地状态） */
async function loadStats(): Promise<void> {
  try {
    const snap = await systemApi.configGet()
    lastBackupAt.value = snap.ints[PREF_KEYS.webdavLastBackup] ?? null
    backupCount.value = snap.ints[PREF_KEYS.webdavBackupCount] ?? 0
  } catch (e) {
    console.warn('[backup] 读备份统计失败', e)
  }
}

// ---------------------------------------------------------------- 本地导出

async function exportLocal(): Promise<void> {
  if (busy.value) return
  exporting.value = true
  progress.value = null
  try {
    const r = await backupApi.backupLocalExport(
      selection.value,
      localPassword.value.trim() || undefined,
    )
    lastExport.value = r
    localPassword.value = ''
    ElMessage.success(`已导出到下载目录：${r.displayName}`)
  } catch (e) {
    ElMessage.error(errorMessage(normalizeError(e)))
  } finally {
    exporting.value = false
  }
}

/** 打开导出文件所在文件夹（用返回的 path，不是我们拼的路径） */
async function revealExport(): Promise<void> {
  const p = lastExport.value?.path
  if (!p) return
  try {
    await reportApi.reportReveal(p)
  } catch (e) {
    ElMessage.error(errorMessage(normalizeError(e)))
  }
}

// ---------------------------------------------------------------- 本地导入

async function pickLocalBackup(): Promise<void> {
  if (busy.value) return
  const path = await pickBackupFile()
  if (!path) return
  await inspectLocal(path, undefined)
}

/** 解析本地包（带密码时用于「密码错了再试」） */
async function inspectLocal(path: string, password: string | undefined): Promise<void> {
  importing.value = true
  try {
    const r = await backupApi.backupInspect(path, password)
    openImportDialog(r, { kind: 'local', path }, password ?? null)
  } catch (e) {
    ElMessage.error(errorMessage(normalizeError(e)))
  } finally {
    importing.value = false
  }
}

// ---------------------------------------------------------------- 云端上传

async function upload(): Promise<void> {
  if (busy.value) return
  if (!hasWebdavPassword.value) {
    ElMessage.warning('请先配置 WebDAV 账号与密码')
    return
  }
  uploading.value = true
  progress.value = null
  try {
    const r = await webdavApi.webdavUpload(selection.value, cloudPassword.value.trim() || undefined)
    cloudPassword.value = ''
    ElMessage.success(`已上传 ${r.name}（${humanSize(r.sizeBytes)}）`)
    await Promise.all([loadRemote(), loadStats()])
  } catch (e) {
    if (isCancelled(normalizeError(e))) return // 事件里已经收尾
    ElMessage.error(errorMessage(normalizeError(e)))
  } finally {
    uploading.value = false
    progress.value = null
  }
}

async function cancel(): Promise<void> {
  try {
    await webdavApi.webdavCancel()
  } catch {
    // 取消本身失败不打扰用户
  }
}

// ---------------------------------------------------------------- 云端导入 / 删除

async function inspectCloud(b: RemoteBackup): Promise<void> {
  if (busy.value) return
  importing.value = true
  progress.value = null
  try {
    const r = await webdavApi.webdavInspect(b.name, undefined)
    openImportDialog(r, { kind: 'cloud', fileName: b.name }, null)
  } catch (e) {
    ElMessage.error(errorMessage(normalizeError(e)))
  } finally {
    importing.value = false
    progress.value = null
  }
}

async function inspectCloudWithPassword(fileName: string, password: string): Promise<void> {
  importing.value = true
  try {
    const r = await webdavApi.webdavInspect(fileName, password)
    openImportDialog(r, { kind: 'cloud', fileName }, password)
  } catch (e) {
    ElMessage.error(errorMessage(normalizeError(e)))
  } finally {
    importing.value = false
  }
}

async function removeRemote(b: RemoteBackup): Promise<void> {
  try {
    await ElMessageBox.confirm(
      `删除云端的 ${b.name}？删除后无法从本应用恢复（网盘回收站可能还能找到）。`,
      '确认删除云端备份',
      { type: 'warning', confirmButtonText: '删除', cancelButtonText: '取消' },
    )
  } catch {
    return
  }
  try {
    await webdavApi.webdavDelete(b.name)
    ElMessage.success('已删除')
    await loadRemote()
  } catch (e) {
    ElMessage.error(errorMessage(normalizeError(e)))
  }
}

// ---------------------------------------------------------------- 导入确认弹窗

function openImportDialog(
  inspect: BackupInspectResult,
  target: { kind: 'local'; path: string } | { kind: 'cloud'; fileName: string },
  password: string | null,
): void {
  importInspect.value = inspect
  importTarget.value = target
  importSource.value = target.kind === 'cloud' ? 'cloud' : 'local'
  unlockedPassword.value = password
  importOpen.value = true
}

/** 弹窗里填了密码要重试解析 */
async function onRetryPassword(password: string): Promise<void> {
  const t = importTarget.value
  if (!t) return
  if (t.kind === 'local') await inspectLocal(t.path, password)
  else await inspectCloudWithPassword(t.fileName, password)
}

async function onConfirmImport(mode: ImportMode): Promise<void> {
  const t = importTarget.value
  if (!t || importing.value) return

  importing.value = true
  progress.value = null
  try {
    const pwd = unlockedPassword.value ?? undefined
    const report =
      t.kind === 'local'
        ? await backupApi.backupLocalImport(t.path, mode, pwd)
        : await webdavApi.webdavDownloadImport(t.fileName, mode, pwd)

    importOpen.value = false
    ElMessage.success(`导入完成（${mode === 'replace' ? '替换' : '合并'}）：${describeSummary(report.summary)}`)
    // 数据被改了 → 云端统计与列表可能也变了
    await loadStats()
  } catch (e) {
    if (isCancelled(normalizeError(e))) return // 事件里已经收尾
    ElMessage.error(errorMessage(normalizeError(e)))
  } finally {
    importing.value = false
    progress.value = null
    unlockedPassword.value = null
  }
}

// ---------------------------------------------------------------- WebDAV 配置

async function onWebDavSaved(): Promise<void> {
  await loadWebDav()
  await loadRemote()
}

async function onClearWebDavPassword(): Promise<void> {
  try {
    await webdavApi.webdavClearPassword()
    hasWebdavPassword.value = false
    ElMessage.success('已清除 WebDAV 密码（配置保留）')
  } catch (e) {
    ElMessage.error(errorMessage(normalizeError(e)))
  }
}
</script>

<template>
  <div class="view">
    <!-- ① 本地备份 -->
    <section class="panel">
      <div class="panel__head">
        <span class="panel__title">本地备份</span>
        <span class="panel__hint">打包成文件放到「下载」目录，离线可用</span>
      </div>

      <div class="panel__body">
        <BackupSelectionForm v-model:selection="selection" />

        <div class="pw">
          <label class="pw__field">
            <span class="pw__label">加密密码（可选）</span>
            <el-input
              v-model="localPassword"
              type="password"
              show-password
              :disabled="busy"
              placeholder="留空则不加密（老版本也能读）"
            />
          </label>
          <p class="pw__hint">
            密码**不会**被保存。设了密码又忘了，这个包就打不开了 ——
            没有找回途径，这是加密的应有之义。
          </p>
        </div>

        <div class="ops">
          <el-button type="primary" :loading="exporting" :disabled="busy" @click="exportLocal()">
            导出备份包
          </el-button>
          <el-button :disabled="busy" @click="pickLocalBackup()">从本机文件导入</el-button>
        </div>

        <!-- 导出结果 -->
        <div v-if="lastExport" class="out">
          <div class="out__row">
            <span class="out__name">{{ lastExport.displayName }}</span>
            <span class="out__size">{{ humanSize(lastExport.sizeBytes) }}</span>
            <button class="link" type="button" @click="revealExport()">打开所在文件夹</button>
          </div>
          <p class="out__summary">{{ describeSummary(lastExport.summary) }}</p>
          <p class="out__path">{{ lastExport.path }}</p>
        </div>
      </div>
    </section>

    <!-- ② 云端备份 -->
    <section class="panel">
      <div class="panel__head">
        <span class="panel__title">云端备份（WebDAV）</span>
      </div>

      <div class="panel__body">
        <WebDavConfigForm
          v-model="webdavConfig"
          :has-password="hasWebdavPassword"
          :disabled="busy"
          @saved="onWebDavSaved"
          @clear-password="onClearWebDavPassword"
        />
      </div>
    </section>

    <!-- ③ 上传与远端列表 -->
    <section class="panel">
      <div class="panel__head">
        <span class="panel__title">上传与恢复</span>
        <span class="panel__hint">上传范围与上面「本地备份」的勾选一致</span>
      </div>

      <div class="panel__body">
        <div class="pw">
          <label class="pw__field">
            <span class="pw__label">备份包加密密码（可选）</span>
            <el-input
              v-model="cloudPassword"
              type="password"
              show-password
              :disabled="busy"
              placeholder="与「本地备份」的密码含义相同"
            />
          </label>
          <p class="pw__hint">
            这里是**备份包密码**，不是上面那张卡里的 WebDAV 账号密码 —— 两者同名但用途不同。
          </p>
        </div>

        <div class="ops">
          <el-button
            type="primary"
            :loading="uploading"
            :disabled="busy || !hasWebdavPassword"
            @click="upload()"
          >
            上传备份
          </el-button>
          <span v-if="!hasWebdavPassword" class="ops__warn">先配置 WebDAV 账号与密码</span>
        </div>

        <ProgressPanel
          :progress="progress"
          :busy="uploading || importing"
          :title="importing ? '正在导入备份' : '正在上传备份'"
          @cancel="cancel()"
        />

        <RemoteFileList
          :backups="remote"
          :total="remoteTotal"
          :loading="remoteLoading"
          :busy="busy"
          :last-backup-at="lastBackupAt"
          :backup-count="backupCount"
          @refresh="loadRemote()"
          @import="inspectCloud"
          @delete="removeRemote"
        />
      </div>
    </section>

    <ImportConfirmDialog
      v-model="importOpen"
      :inspect="importInspect"
      :source="importSource"
      :busy="importing"
      @confirm="onConfirmImport"
      @retry-password="onRetryPassword"
    />
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
  align-items: baseline;
  justify-content: space-between;
  gap: var(--sp-3);
  padding: var(--sp-4) var(--sp-4) 0;
}

.panel__title {
  font-weight: 600;
  color: var(--c-text);
}

.panel__hint {
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
}

.panel__body {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  padding: var(--sp-3) var(--sp-4) var(--sp-4);
}

.pw {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
}

.pw__field {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
  max-width: 420px;
}

.pw__label {
  color: var(--c-text-2);
  font-size: var(--f-size-sm);
}

.pw__hint {
  margin: 0;
  color: var(--c-text-3);
  font-size: var(--f-size-xs);
  line-height: 1.7;
}

.ops {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
  flex-wrap: wrap;
}

.ops__warn {
  color: var(--c-warning);
  font-size: var(--f-size-xs);
}

.out {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: var(--sp-3);
  border-radius: var(--r-card);
  background: var(--c-surface-2);
}

.out__row {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
}

.out__name {
  color: var(--c-text);
  font-size: var(--f-size-sm);
  font-weight: 500;
}

.out__size {
  color: var(--c-text-2);
  font-size: var(--f-size-xs);
}

.out__summary {
  margin: 0;
  color: var(--c-text-2);
  font-size: var(--f-size-xs);
}

.out__path {
  margin: 0;
  color: var(--c-text-3);
  font-family: var(--f-mono);
  font-size: var(--f-size-xs);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
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

.link:hover {
  text-decoration: underline;
}
</style>
