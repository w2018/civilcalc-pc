import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import AutoImport from 'unplugin-auto-import/vite'
import Components from 'unplugin-vue-components/vite'
import { ElementPlusResolver } from 'unplugin-vue-components/resolvers'
import { fileURLToPath, URL } from 'node:url'

// Tauri 期望固定端口；失败即报错，不要静默换端口
const host = process.env.TAURI_DEV_HOST

export default defineConfig({
  plugins: [
    vue(),

    /**
     * Element Plus **按需引入**。
     *
     * 之前是 `app.use(ElementPlus)` + `element-plus/dist/index.css` 全量引入，
     * 单独成块后仍有 915 kB（gzip 295 kB）。实际只用了 7 个组件
     * （input / button / tooltip / skeleton / result / checkbox / autocomplete）
     * 与 2 个函数式 API（ElMessage / ElMessageBox），整体引入浪费约 700 kB。
     *
     * 两个插件分工：
     * - `Components` 处理**模板标签**（`<el-input>` 等），自动 import 组件 + 其 CSS
     * - `AutoImport` 处理**函数式 API**（`ElMessage` 等），否则要手写 import
     *   且**样式不会自动带上**（这是最容易漏的一处）
     *
     * ⚠️ `dts` 必须落在 `src/` 内 —— `tsconfig.json` 的 `include` 只覆盖
     * `src/**`，生成到项目根目录会导致 `vue-tsc` 报「找不到名称 ElMessage」。
     * 这两个 `.d.ts` 是**生成物**，首次 `vite build` / `vite dev` 时产出。
     */
    AutoImport({
      resolvers: [ElementPlusResolver()],
      dts: 'src/types/auto-imports.d.ts',
    }),
    Components({
      resolvers: [ElementPlusResolver()],
      dts: 'src/types/components.d.ts',
    }),
  ],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  // Vite 选项参考 https://v2.tauri.app/start/frontend/vite/
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: 'ws', host, port: 5174 } : undefined,
    watch: {
      // 不要监听 src-tauri（Rust 侧由 cargo 自己管）
      ignored: ['**/src-tauri/**', '**/crates/**', '**/target/**'],
    },
  },
  envPrefix: ['VITE_', 'TAURI_ENV_*'],
  build: {
    // Windows 目标用 chrome105；避免为旧浏览器降级
    target: 'chrome105',
    minify: !process.env.TAURI_ENV_DEBUG ? 'esbuild' : false,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
    /**
     * 按需引入后 element-plus 块从 915 kB 降到约 100 kB。
     *
     * 保留 500 kB 的默认阈值即可（见下方 `manualChunks` 的说明）。
     */
    chunkSizeWarningLimit: 500,
    rollupOptions: {
      output: {
        /**
         * 手动分包：把体积大且**极少变动**的依赖单独拆出来。
         *
         * ## ⚠️ 不要给 `element-plus` 分包（实测会抵消 tree-shaking）
         *
         * 无论数组式还是函数式，只要把 `element-plus` 列进 `manualChunks`，
         * Rollup 就会把**整包**纳入该块 —— 未用到的 `ElTable` / `ElUpload` /
         * `ElDatePicker` 全都会出现在产物里。
         *
         * 实测数据（同一份代码，只改这一处）：
         *
         * | 配置 | 产物总 JS |
         * |---|---|
         * | `manualChunks` 含 element-plus（数组式） | 1065 kB |
         * | `manualChunks` 含 element-plus（函数式） | 1065 kB |
         * | **不给 element-plus 分包** | **239 kB** |
         *
         * 根因：`element-plus/es/index.mjs` 是带副作用的 barrel 文件
         * （每个组件都 `import './style/css'`）。一旦这个模块被显式指定
         * 进某个 chunk，Rollup 就不再对它的 re-export 做死代码消除。
         *
         * 代价是 element-plus 与业务代码同处 `index` 块 —— 桌面应用走本地
         * 文件，缓存收益本来就小，换来 4.4 倍的体积下降是划算的。
         */
        manualChunks(id) {
          if (!id.includes('node_modules')) return undefined
          // Vue 生态（精确匹配，避免误伤 @vueuse 等）
          if (/[\\/]node_modules[\\/](vue|vue-router|pinia|@vue)[\\/]/.test(id)) return 'vue-vendor'
          return undefined
        },
      },
    },
  },
})
