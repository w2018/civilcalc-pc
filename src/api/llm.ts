/**
 * 组 8：LLM 配置与密钥。
 *
 * 见 docs/08-IPC契约.md §3 组 8。
 *
 * ## 🔒 凭据铁律
 *
 * - `llmListProfiles()` 的返回**不含 Key**（`LlmProfile` 类型本身就没有该字段）
 * - Key 只经 {@link llmSetApiKey} / {@link llmSaveProfile} 进系统凭据管理器
 * - {@link llmConfigExport} 传 `true` 是**唯一**会吐明文 Key 的路径 ——
 *   必须先弹二次确认，默认传 `false`
 */

import { invoke } from './invoke'
import type {
  LlmConfig,
  LlmConfigImportReport,
  LlmProfile,
  LlmTestResult,
} from '@/types/llm'

/** 读全部模型配置（**不含 Key**） */
export function llmListProfiles(): Promise<LlmConfig> {
  return invoke<LlmConfig>('llm_list_profiles')
}

/** 保存档位；`apiKey` 为 `undefined` 时**不动**已有 Key */
export function llmSaveProfile(profile: LlmProfile, apiKey?: string): Promise<void> {
  return invoke<void>('llm_save_profile', { profile, apiKey })
}

/** 只更新档位元数据，**不动 Key** */
export function llmUpdateProfile(profile: LlmProfile): Promise<void> {
  return invoke<void>('llm_update_profile', { profile })
}

/** 设为活跃档位；档位不存在 → `notFound` */
export function llmSetActive(id: string): Promise<void> {
  return invoke<void>('llm_set_active', { id })
}

/**
 * 删除档位（**同时删系统凭据里的配置与 Key**）。
 *
 * 删掉的是活跃档位时，后端会**自动回落到其他可用档位**。
 */
export function llmDeleteProfile(id: string): Promise<void> {
  return invoke<void>('llm_delete_profile', { id })
}

/** 单独设置某档位的 API Key */
export function llmSetApiKey(profileId: string, apiKey: string): Promise<void> {
  return invoke<void>('llm_set_api_key', { profileId, apiKey })
}

export function llmHasApiKey(profileId: string): Promise<boolean> {
  return invoke<boolean>('llm_has_api_key', { profileId })
}

/** 删除某档位的 Key（**幂等**） */
export function llmDeleteApiKey(profileId: string): Promise<void> {
  return invoke<void>('llm_delete_api_key', { profileId })
}

/**
 * 连通性测试（发最小请求验证连通性与 Key）。
 *
 * ⚠️ **尚未实现**（依赖 P4-2 的 LLM 客户端）：后端会返回
 * `invalidArgument`，消息里说明"档位与 Key 均已就绪，仅缺客户端"。
 * 前端应把这种情况与"真的连不通"区分开。
 */
export function llmTestConnection(profileId: string): Promise<LlmTestResult> {
  return invoke<LlmTestResult>('llm_test_connection', { profileId })
}

/**
 * 导出配置 JSON 包。
 *
 * ⚠️ `includeApiKeys: true` 会**吐出明文 Key** ——
 * 调用前必须弹二次确认。
 */
export function llmConfigExport(includeApiKeys = false): Promise<string> {
  return invoke<string>('llm_config_export', { includeApiKeys })
}

/**
 * 导入配置 JSON 包。
 *
 * 包里缺 `apiKeys` 时**不动本机 Key**。
 */
export function llmConfigImport(json: string): Promise<LlmConfigImportReport> {
  return invoke<LlmConfigImportReport>('llm_config_import', { json })
}
