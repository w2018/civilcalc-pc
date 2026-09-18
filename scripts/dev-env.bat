@echo off
REM ============================================================================
REM  dev-env.bat -- Run a command inside the MSVC x64 toolchain environment.
REM
REM  Why this exists (two separate Windows + Git Bash problems):
REM
REM   1) link.exe shadowing
REM      GNU coreutils' link.exe (C:\Program Files\coreutils\bin\link.exe)
REM      shadows MSVC's link.exe in PATH. Cargo then invokes the wrong tool:
REM          link: extra operand '...rcgu.o'
REM
REM   2) vcvars64.bat cannot always be used
REM      vcvars64.bat shells out to reg.exe to discover the Windows SDK.
REM      In restricted/sandboxed environments reg.exe may be blocked, leaving
REM      PATH set but LIB unset, which fails later with:
REM          LINK : fatal error LNK1181: cannot open input file 'kernel32.lib'
REM
REM  This script therefore sets PATH / LIB / INCLUDE directly from the
REM  filesystem (no reg.exe), picking the highest installed versions.
REM
REM  Usage (from Git Bash):
REM     ./scripts/dev-env.bat cargo check -p civilcalc-core
REM     ./scripts/dev-env.bat cargo test --workspace
REM     ./scripts/dev-env.bat cargo build -p civilcalc-pc --release
REM
REM  Usage (from cmd / PowerShell):
REM     scripts\dev-env.bat cargo check --workspace
REM ============================================================================

setlocal enabledelayedexpansion

REM ---------------------------------------------------------------- 1. VS path
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if not exist "%VSWHERE%" set "VSWHERE=%ProgramFiles%\Microsoft Visual Studio\Installer\vswhere.exe"
if not exist "%VSWHERE%" set "VSWHERE=C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe"
if not exist "%VSWHERE%" set "VSWHERE=D:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe"

set "VSPATH="
if exist "%VSWHERE%" (
    for /f "usebackq tokens=*" %%i in (`"%VSWHERE%" -latest -products * -property installationPath`) do set "VSPATH=%%i"
)

REM Fallback: known Build Tools locations (VS 2022 / VS 18 layout)
if not defined VSPATH if exist "D:\ProgramData\Microsoft\MicrosoftVisualStudio\18\BuildTools\VC" set "VSPATH=D:\ProgramData\Microsoft\MicrosoftVisualStudio\18\BuildTools"
if not defined VSPATH if exist "%ProgramFiles%\Microsoft Visual Studio\2022\BuildTools\VC" set "VSPATH=%ProgramFiles%\Microsoft Visual Studio\2022\BuildTools"
if not defined VSPATH if exist "%ProgramFiles%\Microsoft Visual Studio\2022\Community\VC" set "VSPATH=%ProgramFiles%\Microsoft Visual Studio\2022\Community"

if not defined VSPATH (
    echo [ERROR] Visual Studio / Build Tools not found.
    echo         Install "Desktop development with C++":
    echo         https://visualstudio.microsoft.com/visual-cpp-build-tools/
    exit /b 1
)

REM ------------------------------------------------------- 2. MSVC toolset x64
set "MSVCROOT=%VSPATH%\VC\Tools\MSVC"
set "MSVCVER="
for /f "delims=" %%d in ('dir /b /ad /o-n "%MSVCROOT%" 2^>nul') do (
    if not defined MSVCVER set "MSVCVER=%%d"
)
if not defined MSVCVER (
    echo [ERROR] No MSVC toolset found under: %MSVCROOT%
    exit /b 1
)

set "VCBIN=%MSVCROOT%\%MSVCVER%\bin\Hostx64\x64"
set "VCLIB=%MSVCROOT%\%MSVCVER%\lib\x64"
set "VCINC=%MSVCROOT%\%MSVCVER%\include"

if not exist "%VCBIN%\link.exe" (
    echo [ERROR] link.exe not found at: %VCBIN%
    exit /b 1
)

REM ------------------------------------------------------- 3. Windows SDK 10
set "SDKROOT=%ProgramFiles(x86)%\Windows Kits\10"
if not exist "%SDKROOT%" set "SDKROOT=%ProgramFiles%\Windows Kits\10"
if not exist "%SDKROOT%" set "SDKROOT=C:\Program Files (x86)\Windows Kits\10"
if not exist "%SDKROOT%" set "SDKROOT=D:\Program Files (x86)\Windows Kits\10"

if not exist "%SDKROOT%\Lib" (
    echo [ERROR] Windows SDK not found under: %SDKROOT%
    echo         Install the "Windows 10/11 SDK" component.
    exit /b 1
)

set "SDKVER="
for /f "delims=" %%d in ('dir /b /ad /o-n "%SDKROOT%\Lib" 2^>nul') do (
    if not defined SDKVER set "SDKVER=%%d"
)
if not defined SDKVER (
    echo [ERROR] No Windows SDK version found under: %SDKROOT%\Lib
    exit /b 1
)

set "SDKLIB=%SDKROOT%\Lib\%SDKVER%"
set "SDKINC=%SDKROOT%\Include\%SDKVER%"

if not exist "%SDKLIB%\um\x64\kernel32.lib" (
    echo [ERROR] kernel32.lib not found at: %SDKLIB%\um\x64
    exit /b 1
)

REM ------------------------------------------------------------------ 4. env
REM PATH first so MSVC's link.exe wins over coreutils' link.exe
set "PATH=%VCBIN%;%PATH%"
REM LIB: MSVC CRT + UCRT + UM import libraries
set "LIB=%VCLIB%;%SDKLIB%\ucrt\x64;%SDKLIB%\um\x64"
set "INCLUDE=%VCINC%;%SDKINC%\ucrt;%SDKINC%\um;%SDKINC%\shared"

if defined DEVENV_VERBOSE (
    echo [dev-env] VS      = %VSPATH%
    echo [dev-env] MSVC    = %MSVCVER%
    echo [dev-env] SDK     = %SDKVER%
    echo [dev-env] LINK    = %VCBIN%\link.exe
    echo [dev-env] LIB     = %LIB%
)

REM ------------------------------------------------------------------ 5. run
%*
exit /b %ERRORLEVEL%
