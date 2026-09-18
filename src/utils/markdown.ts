/**
 * 轻量 Markdown 渲染 + HTML 白名单过滤。
 *
 * 见 docs/05-项目开发方案.md §2.3.2「MarkdownText」。
 *
 * ## 三档高亮是**提示词契约**，不是随手定的
 *
 * `crates/civilcalc-llm/src/prompts.rs` 要求模型：
 * - `**加粗**` 标关键依据 / 结论 / 系数含义
 * - `` `行内代码` `` 标变量符号 / 系数数值 / 单位
 * - `==高亮==` 标每步最核心的结果（金色荧光底）
 *
 * 前两档 `marked` 原生支持；`==…==` 是**自定义语法**，
 * 在这里预处理成 `<mark>` 再交给 `marked`（白名单里有 `MARK`）。
 *
 * ## 🔴 必须过滤：AI 输出是不可信输入
 *
 * 模型（或提示注入）完全可能吐出 `<script>` / `<img onerror=…>`。
 * 渲染进 `v-html` 就等于在自己的 WebView 里执行它。
 * 所以走「**白名单**」而不是黑名单：只留我们要的标签，
 * 属性一律剥光（`a` 只留 `href`/`title`，且必须是 http/https/mailto）。
 *
 * `DOMParser` 解析 `text/html` **不会执行脚本**，用它做过滤是安全的。
 */

import { marked } from 'marked'

/** 允许保留的标签（其余一律「脱壳」保留文字） */
const ALLOWED_TAGS = new Set([
  'P',
  'BR',
  'HR',
  'STRONG',
  'B',
  'EM',
  'I',
  'DEL',
  'S',
  'MARK',
  'CODE',
  'PRE',
  'UL',
  'OL',
  'LI',
  'BLOCKQUOTE',
  'H1',
  'H2',
  'H3',
  'H4',
  'H5',
  'H6',
  'A',
  'TABLE',
  'THEAD',
  'TBODY',
  'TR',
  'TH',
  'TD',
  'SPAN',
])

/** 直接**整段删除**的标签（连内容一起丢） */
const DROP_TAGS = new Set([
  'SCRIPT',
  'STYLE',
  'IFRAME',
  'FRAME',
  'FRAMESET',
  'OBJECT',
  'EMBED',
  'APPLET',
  'LINK',
  'META',
  'BASE',
  'FORM',
  'INPUT',
  'BUTTON',
  'SELECT',
  'TEXTAREA',
  'SVG',
  'MATH',
  'TEMPLATE',
  'NOSCRIPT',
  'AUDIO',
  'VIDEO',
  'SOURCE',
  'TRACK',
  'CANVAS',
])

/** `a` 的 `href` 只放行这三种协议 */
const SAFE_HREF = /^(https?:|mailto:)/i

/** 用子节点替换掉元素本身（保留其内容） */
function unwrap(el: Element): void {
  const parent = el.parentNode
  if (!parent) return
  while (el.firstChild) parent.insertBefore(el.firstChild, el)
  parent.removeChild(el)
}

/**
 * 白名单过滤。
 *
 * 输入是 `marked` 产出的 HTML（或任何 HTML 片段），输出是**只含安全标签与零属性**的 HTML。
 */
export function sanitizeHtml(html: string): string {
  // 无 DOM 环境（理论上不会走到：只在 WebView 里渲染）
  if (typeof DOMParser === 'undefined') return ''

  const doc = new DOMParser().parseFromString(`<div id="__root">${html}</div>`, 'text/html')
  const root = doc.getElementById('__root')
  if (!root) return ''

  // 先取快照：脱壳会改变树结构，边遍历边改会漏元素
  for (const el of Array.from(root.querySelectorAll('*'))) {
    const tag = el.tagName.toUpperCase()

    if (DROP_TAGS.has(tag)) {
      el.remove()
      continue
    }
    if (!ALLOWED_TAGS.has(tag)) {
      unwrap(el)
      continue
    }

    // 属性一律剥光，只给 `a` 留 href/title
    const href = tag === 'A' ? el.getAttribute('href') : null
    const title = tag === 'A' ? el.getAttribute('title') : null
    for (const attr of Array.from(el.attributes)) {
      el.removeAttribute(attr.name)
    }
    if (tag === 'A') {
      if (href && SAFE_HREF.test(href.trim())) {
        el.setAttribute('href', href.trim())
        // 外链一律新窗口 + 断开 opener
        el.setAttribute('target', '_blank')
        el.setAttribute('rel', 'noopener noreferrer')
      }
      if (title) el.setAttribute('title', title)
    }
  }

  return root.innerHTML
}

/**
 * `==高亮==` → `<mark>高亮</mark>`。
 *
 * 只认**同一行内**的 `==…==`（不跨换行、内容非空且不含 `=`）——
 * 跨行匹配会把两个无关的 `==` 之间整段吞掉。
 *
 * ⚠️ 先做转义再做替换会引入实体问题，所以这里**只做替换**，
 * 安全由后续的 `sanitizeHtml` 兜底。
 */
export function preprocessHighlight(text: string): string {
  return text.replace(/==([^=\n]+)==/g, '<mark>$1</mark>')
}

/**
 * Markdown → 安全 HTML。
 *
 * `breaks: true` —— 单个换行也当换行。模型输出常把分点写成一行一条，
 * 若按标准 Markdown（单换行 = 空格）会把所有分点挤成一整段。
 *
 * 注意：**不要**把这里的结果直接塞 `v-html` 之外的场景（如写回数据库），
 * 它是有损的展示层产物。
 */
export function renderMarkdown(text: string): string {
  if (!text) return ''
  const withMarks = preprocessHighlight(text)
  const html = marked.parse(withMarks, { breaks: true, gfm: true }) as string
  return sanitizeHtml(html)
}

/**
 * 纯文本预览：去掉全部 Markdown 标记。
 *
 * 用于列表行副标题（如公式 desc）—— 那里不想渲染 HTML，只想显示干净文字。
 */
export function stripMarkdown(text: string): string {
  if (!text) return ''
  return text
    .replace(/==([^=\n]+)==/g, '$1')
    .replace(/\*\*([^*]+)\*\*/g, '$1')
    .replace(/`([^`]+)`/g, '$1')
    .replace(/^\s*#{1,6}\s+/gm, '')
    .replace(/^\s*[-*+]\s+/gm, '')
    .trim()
}
