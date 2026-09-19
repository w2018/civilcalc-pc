; =============================================================================
;  NSIS 卸载钩子 —— 加固「删除应用程序数据」
;
;  由 `tauri.conf.json` 的 `bundle.windows.nsis.installerHooks` 引入。
;
;  ## 为什么要这个钩子
;
;  Tauri 模板在用户勾选「删除应用程序数据」时执行的是：
;
;      RmDir /r "$APPDATA\${BUNDLEID}"
;      RmDir /r "$LOCALAPPDATA\${BUNDLEID}"
;
;  NSIS 的 `RmDir /r` 是**尽力而为**的 —— 遇到「文件被占用」或「只读文件」
;  会**静默失败**，既不报错也不重试。于是用户明明勾了删除，数据却还在。
;
;  最容易卡住的是 WebView2 的 `EBWebView\`：它位于
;  `$LOCALAPPDATA\${BUNDLEID}` 下、几千个小文件，应用进程退出后
;  句柄不保证立刻释放。SQLite 的 `-wal` / `-shm` 也有同样的问题。
;
;  ## 这个钩子做什么
;
;  它跑在模板那段删除**之后**（`NSIS_HOOK_POSTUNINSTALL`），三件事：
;    1. 再删一遍，并**重试若干次**（占用通常是瞬时的）
;    2. 每次重试之间 Sleep 一下，给系统释放句柄的时间
;    3. 重试完还是删不掉就**明确弹窗告诉用户**该手动删哪个目录 ——
;       宁可打扰一下，也不能让用户以为已经清干净了
;
;  ⚠️ `$DeleteAppDataCheckboxState` 是主脚本里的全局 Var（在卸载确认页
;     的 `un.ConfirmLeave` 里读入），钩子被插入的位置在它之后，所以能读到。
; =============================================================================

Var CC_Retries

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    StrCpy $CC_Retries 0

    cc_retry:
      RmDir /r "$APPDATA\${BUNDLEID}"
      RmDir /r "$LOCALAPPDATA\${BUNDLEID}"

      IntOp $CC_Retries $CC_Retries + 1
      ${If} $CC_Retries >= 5
        Goto cc_report
      ${EndIf}

      ; 两处都空了才算删干净；还有残留就等一会儿再来一轮
      IfFileExists "$APPDATA\${BUNDLEID}\*.*" cc_wait
      IfFileExists "$LOCALAPPDATA\${BUNDLEID}\*.*" cc_wait
      Goto cc_report

      cc_wait:
        Sleep 500
        Goto cc_retry

    cc_report:
      IfFileExists "$APPDATA\${BUNDLEID}\*.*" cc_report_appdata
      IfFileExists "$LOCALAPPDATA\${BUNDLEID}\*.*" cc_report_local
      Goto cc_done

      cc_report_appdata:
        MessageBox MB_ICONEXCLAMATION|MB_OK \
          "应用数据未能完全删除（可能有文件被占用）。$\r$\n请手动删除目录：$\r$\n$APPDATA\${BUNDLEID}"
        Goto cc_done

      cc_report_local:
        MessageBox MB_ICONEXCLAMATION|MB_OK \
          "应用数据未能完全删除（可能有文件被占用）。$\r$\n请手动删除目录：$\r$\n$LOCALAPPDATA\${BUNDLEID}"

    cc_done:
  ${EndIf}
!macroend
