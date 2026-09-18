/**
 * 剪贴板封装。
 *
 * ## 为什么不直接用 `navigator.clipboard`
 *
 * WebView 里 `navigator.clipboard` 需要**安全上下文**（https 或 localhost）。
 * Tauri 打包后页面走 `tauri://localhost`，属于安全上下文，通常可用；
 * 但开发时若通过 IP 访问（如 `http://192.168.x.x:5173`）就会失败。
 *
 * 因此加一层 `document.execCommand('copy')` 兜底 —— 它在 WebView2 里仍有效。
 *
 * ## 为什么返回布尔而不是抛错
 *
 * 复制失败是**可恢复的小事**：调用方弹个「复制失败，请手动选中」即可，
 * 不该让整个操作链断掉（`await` 一个 reject 会让后续代码不执行）。
 */

/**
 * 写入剪贴板。
 *
 * @returns 是否成功
 */
export async function copyText(text: string): Promise<boolean> {
  if (!text) return false

  // 首选异步 API
  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text)
      return true
    } catch {
      // 权限被拒 / 非安全上下文 → 走兜底
    }
  }

  return copyViaExecCommand(text)
}

/** `execCommand` 兜底（需要临时 textarea） */
function copyViaExecCommand(text: string): boolean {
  const ta = document.createElement('textarea')
  ta.value = text
  // 放到视口内但不可见 —— `display: none` 会让 select() 失效
  ta.style.position = 'fixed'
  ta.style.top = '0'
  ta.style.left = '0'
  ta.style.opacity = '0'
  ta.setAttribute('readonly', '')
  document.body.appendChild(ta)

  try {
    ta.select()
    return document.execCommand('copy')
  } catch {
    return false
  } finally {
    document.body.removeChild(ta)
  }
}
