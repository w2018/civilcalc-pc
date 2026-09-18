/**
 * 组 14：WebDAV 云端备份（11 个命令）。
 *
 * 见 docs/08-IPC契约.md §3 组 14、docs/05-项目开发方案.md §1.3.2。
 *
 * ## 🔴 两个 `password` 参数含义**不同**
 *
 * | 函数 | `password` 是 |
 * |---|---|
 * | {@link webdavConfigSave} | **WebDAV 账号密码**（存 keyring） |
 * | {@link webdavUpload} / {@link webdavInspect} / {@link webdavDownloadImport} | **备份包加密密码**（不存） |
 *
 * 传错不会报错，只会「连不上」或「打不开」。
 *
 * ## 密码只进 keyring
 *
 * 契约要求：**密码提交后前端立即清空输入框**。
 * 所以 {@link webdavConfigSave} 的 `password` 传空串是常态
 * （= 不改动已保存的密码），后端也按这个语义处理。
 *
 * ## 进度事件
 *
 * 上传/下载过程发 {@link BACKUP_PROGRESS_EVENT}（`{ stage, current, total }`），
 * 取消发 {@link BACKUP_CANCELLED_EVENT}。订阅后**务必在 `onUnmounted` 里
 * `unlisten()`**，否则会泄漏监听器。
 */

import { invoke } from './invoke'
import type {
  BackupInspectResult,
  BackupSelection,
  ImportMode,
  ImportReport,
  RemoteListResult,
  UploadResult,
  WebDavConfig,
  WebDavTestResult,
} from '@/types/backup'

// ---------------------------------------------------------------------------
// 配置与密码
// ---------------------------------------------------------------------------

/** 读 WebDAV 配置（**不含密码**）。未配置时返回坚果云默认值。 */
export function webdavConfigGet(): Promise<WebDavConfig> {
  return invoke<WebDavConfig>('webdav_config_get')
}

/**
 * 保存 WebDAV 配置。
 *
 * @param password **WebDAV 账号密码**。⚠️ 传空串或 `undefined` = **不改动已有密码**
 *   （不是「设成空密码」）—— 前端提交后清空输入框是常态。
 */
export function webdavConfigSave(
  config: WebDavConfig,
  password?: string,
): Promise<void> {
  return invoke<void>('webdav_config_save', { config, password })
}

/** 是否已保存 WebDAV 密码（**不返回密码本身**） */
export function webdavHasPassword(): Promise<boolean> {
  return invoke<boolean>('webdav_has_password')
}

/** 清除已保存的 WebDAV 密码（配置保留） */
export function webdavClearPassword(): Promise<void> {
  return invoke<void>('webdav_clear_password')
}

// ---------------------------------------------------------------------------
// 连接与列表
// ---------------------------------------------------------------------------

/**
 * 测试连接（MKCOL + PROPFIND）。
 *
 * ## 两个参数都是可选的（与契约的「无入参」不同，纯增量）
 *
 * 用户流程是「填地址 → **测试连接** → 保存」，也就是**保存之前**就要能测。
 * 传了就用传进来的（弹窗里当前填的值），没传就回落已保存的配置与 keyring 密码。
 *
 * ## 🔴 两类失败要分开处理
 *
 * - **本地配置问题**（地址不是 https / 没密码）→ **抛错**
 *   （`invalidArgument` / `unauthorized`）→ 引导用户去改配置
 * - **连不上**（网络、鉴权被拒、证书）→ 返回 `ok: false` → 提示查网络
 */
export function webdavTestConnection(
  config?: WebDavConfig,
  password?: string,
): Promise<WebDavTestResult> {
  return invoke<WebDavTestResult>('webdav_test_connection', { config, password })
}

/**
 * 列远端备份（按时间倒序，最多 {@link WEBDAV_LIST_LIMIT} 条）。
 *
 * ⚠️ 返回值里的 `total` 是**服务器上的总数**（截断前）——
 * 界面据此提示「还有 N 条更早的」。
 */
export function webdavList(): Promise<RemoteListResult> {
  return invoke<RemoteListResult>('webdav_list')
}

// ---------------------------------------------------------------------------
// 上传 / 下载 / 删除 / 取消
// ---------------------------------------------------------------------------

/**
 * 打包并上传到 WebDAV。
 *
 * 过程中发 {@link BACKUP_PROGRESS_EVENT}；可 {@link webdavCancel} 取消。
 *
 * ⚠️ 取消时抛的是 `cancelled` 错误（不是「成功」）——
 * 只 `await` 而没监听取消事件时，**必须**用 `isCancelled(e)` 判别并静默收尾，
 * 否则会把「用户取消」显示成「上传成功」。
 *
 * @param password **备份包加密密码**
 */
export function webdavUpload(
  selection: BackupSelection,
  password?: string,
): Promise<UploadResult> {
  return invoke<UploadResult>('webdav_upload', { selection, password })
}

/**
 * 只解析云端备份包的清单（**不导入**，供二次确认）。
 *
 * 会把包下载到本机临时目录并留在那里，供随后的 {@link webdavDownloadImport}
 * 复用 —— **不会下载两次**。
 *
 * @param fileName 远端文件名（来自 {@link webdavList}，后端会校验格式）
 * @param password **备份包加密密码**
 */
export function webdavInspect(
  fileName: string,
  password?: string,
): Promise<BackupInspectResult> {
  return invoke<BackupInspectResult>('webdav_inspect', { fileName, password })
}

/**
 * 下载云端备份并导入。
 *
 * 若刚 {@link webdavInspect} 过同一个包（缓存命中），**跳过下载**直接导入。
 *
 * @param mode `'merge'`（默认）或 `'replace'`
 * @param password **备份包加密密码**
 */
export function webdavDownloadImport(
  fileName: string,
  mode: ImportMode,
  password?: string,
): Promise<ImportReport> {
  return invoke<ImportReport>('webdav_download_import', {
    fileName,
    mode,
    password,
  })
}

/** 删除远端备份（**已不存在也算成功**） */
export function webdavDelete(fileName: string): Promise<void> {
  return invoke<void>('webdav_delete', { fileName })
}

/**
 * 取消当前上传/下载（**立即断流**，不是「跑完再丢」）。
 *
 * 后端置位取消标志，网络读循环每读一个分块检查一次，命中即断开连接。
 */
export function webdavCancel(): Promise<void> {
  return invoke<void>('webdav_cancel')
}
