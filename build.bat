@echo off
rem ---------------------------------------------------------------------------
rem  flatground - Windows build
rem
rem  Double-click this file (or run it from a command prompt). It:
rem    1. updates this folder from GitHub (git pull), when it can
rem    2. builds the Wii game (Rust + devkitPPC)
rem    3. packages it for the Homebrew Channel in dist\apps\flatground
rem       (boot.dol + meta.xml). Copy that folder into the SD card's apps folder.
rem
rem  Needs one of:
rem    - devkitPro with "Wii Development" ticked (github.com/devkitPro/installer),
rem      plus Rust (this script offers to install it), or
rem    - Docker Desktop (it downloads the devkitPro image and Rust the first time).
rem  Git for Windows is used for the update step when it's installed (optional).
rem
rem    build.bat            update, build and package
rem    build.bat noupdate   the same without updating
rem    build.bat clean      delete the build output
rem    build.bat nopause    don't wait for a key at the end
rem    build.bat dolphin    build, then start the game in Dolphin (tools\dolphin.ps1)
rem  Switches combine (build.bat noupdate nopause).
rem
rem  Optional settings (set them before running, e.g. set FG_SD=E:):
rem    FG_SD=E:                   also copy the app to E:\apps\flatground
rem    FG_DEVKITPRO=D:\devkitPro  where devkitPro is (default: searched on every drive)
rem    FG_ASSUME_YES=1            install Rust without asking
rem    FG_NO_DOCKER=1             never fall back to Docker
rem    FG_ISO=D:\skate3.iso       your Skate 3 disc image (default: the one .iso found next to
rem                               this folder or one folder down; or iso=<path> in fg_local.txt)
rem    FG_PARK=MegaPark           which park to convert (default BlackBoxPark)
rem    FG_NO_ASSETS=1             don't convert anything from the disc
rem    FG_DOLPHIN=C:\...\Dolphin.exe   where Dolphin is, if it isn't found by itself
rem
rem  The approach (devkitPro's msys2 shell, Docker fallback) follows modernwiifare's build.bat.
rem ---------------------------------------------------------------------------
setlocal
cd /d "%~dp0"

set "TARGET=package"
set "UPDATE=1"
rem Wait for a key at the end only when double-clicked (no arguments).
set "PAUSE=1"
set "DOLPHIN=0"
if not "%~1"=="" set "PAUSE=0"
for %%A in (%*) do (
    if /i "%%~A"=="clean" set "TARGET=clean"
    if /i "%%~A"=="clean" set "UPDATE=0"
    if /i "%%~A"=="noupdate" set "UPDATE=0"
    if /i "%%~A"=="nopause" set "PAUSE=0"
    if /i "%%~A"=="dolphin" set "DOLPHIN=1"
    if /i "%%~A"=="showargs" set "SHOWARGS=1"
)
if defined SHOWARGS (
    echo TARGET=%TARGET% UPDATE=%UPDATE% PAUSE=%PAUSE% DOLPHIN=%DOLPHIN% SD=%FG_SD%
    endlocal
    exit /b 0
)

rem ---- 1. update from GitHub --------------------------------------------------
if "%UPDATE%"=="0" goto build
rem Prefer Git for Windows: devkitPro's installer can put its own git earlier on the PATH.
set "GIT=git"
if exist "%ProgramFiles%\Git\cmd\git.exe" (
    set "GIT=%ProgramFiles%\Git\cmd\git.exe"
) else (
    where git >nul 2>nul
    if errorlevel 1 (
        echo [update] Git isn't installed, so not updating. Get it from git-scm.com.
        goto build
    )
)
if not exist ".git" (
    echo [update] This folder isn't a git clone ^(downloaded as a ZIP?^), so not updating.
    echo          For automatic updates: git clone https://github.com/slanginbeans/skatewii
    goto build
)
echo [update] Getting the latest version...
set "GIT_TERMINAL_PROMPT=0"
"%GIT%" pull --ff-only
if errorlevel 1 (
    echo.
    echo [update] Couldn't update, so building what's here. Usual reasons:
    echo   - No internet.
    echo   - Files in this folder were edited. To throw those edits away:
    echo       "%GIT%" reset --hard origin/main
    echo.
)

:build
rem ---- 2/3. build and package -------------------------------------------------
rem devkitPro's Windows installer puts its tools under X:\devkitPro\msys2 (any drive), and its
rem make has to run inside that shell, so it usually isn't on the normal PATH.
set "DKP="
if defined FG_DEVKITPRO if exist "%FG_DEVKITPRO%\msys2\usr\bin\bash.exe" set "DKP=%FG_DEVKITPRO%"
if not defined DKP for %%D in (C D E F G H I J K L M N O P Q R S T U V W X Y Z) do (
    if not defined DKP if exist "%%D:\devkitPro\msys2\usr\bin\bash.exe" set "DKP=%%D:\devkitPro"
)
rem The helper script, with forward slashes for devkitPro's shell.
set "WINBUILD=%~dp0tools\winbuild.sh"
set "WINBUILD=%WINBUILD:\=/%"
if defined DKP (
    if not exist "%DKP%\devkitPPC\bin" (
        echo [build] devkitPro is installed but without the Wii tools.
        echo         Run the devkitPro installer again and tick "Wii Development".
        goto failed
    )
    if /i not "%TARGET%"=="clean" call :ensure_rust
    if errorlevel 1 goto failed
    echo [build] Using devkitPro in %DKP%: make %TARGET%
    "%DKP%\msys2\usr\bin\bash.exe" -l "%WINBUILD%" %TARGET%
    if errorlevel 1 goto failed
    goto done
)
rem devkitPro set up some other way, with make on the PATH.
if defined DEVKITPPC (
    where make >nul 2>nul
    if not errorlevel 1 (
        if /i not "%TARGET%"=="clean" call :ensure_rust
        if errorlevel 1 goto failed
        set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
        echo [build] Using devkitPro: make -C wii %TARGET%
        make -C wii %TARGET%
        if errorlevel 1 goto failed
        goto done
    )
)

rem Docker Desktop (on the PATH, or where it installs).
if "%FG_NO_DOCKER%"=="1" goto notools
set "DOCKER=docker"
where docker >nul 2>nul
if errorlevel 1 (
    if exist "%ProgramFiles%\Docker\Docker\resources\bin\docker.exe" (
        set "DOCKER=%ProgramFiles%\Docker\Docker\resources\bin\docker.exe"
    ) else (
        goto notools
    )
)
"%DOCKER%" info >nul 2>nul
if errorlevel 1 (
    echo [build] Docker Desktop is installed but not running. Start it, wait until it says
    echo         it's running, then run build.bat again.
    goto failed
)
echo [build] Using Docker ^(devkitpro/devkitppc^): make %TARGET%
echo         The first run also downloads Rust into .toolchain\ ^(a few minutes^).
"%DOCKER%" run --rm -v "%cd%":/src -w /src devkitpro/devkitppc sh tools/docker-build.sh %TARGET%
if errorlevel 1 goto failed
goto done

rem ---- Rust: needed to build the game code ------------------------------------
:ensure_rust
if exist "%USERPROFILE%\.cargo\bin\cargo.exe" exit /b 0
where cargo >nul 2>nul
if not errorlevel 1 exit /b 0
echo.
echo [rust] Rust isn't installed. It's needed to build the game code.
echo        It installs into your user folder ^(%USERPROFILE%\.cargo^), no admin needed.
if "%FG_ASSUME_YES%"=="1" goto rust_yes
choice /c YN /m "[rust] Install Rust now"
if errorlevel 2 (
    echo [rust] Not installing. Get it later from https://rustup.rs and run build.bat again.
    exit /b 1
)
rem The GNU host toolchain brings its own linker, so Visual Studio isn't needed.
:rust_yes
set "RUSTUP_INIT=%TEMP%\rustup-init.exe"
echo [rust] Downloading the Rust installer...
curl -sSfL -o "%RUSTUP_INIT%" https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-gnu/rustup-init.exe
if errorlevel 1 (
    echo [rust] Download failed. Install it by hand from https://rustup.rs
    exit /b 1
)
"%RUSTUP_INIT%" -y --default-host x86_64-pc-windows-gnu --profile minimal --default-toolchain none
if errorlevel 1 (
    echo [rust] The installer failed. Install it by hand from https://rustup.rs
    exit /b 1
)
del "%RUSTUP_INIT%" >nul 2>nul
echo [rust] Installed. The first build downloads the exact Rust version this project pins.
exit /b 0

:notools
echo.
echo [build] No Wii build tools found on this PC. The easiest fix:
echo.
echo   1. Download the devkitPro installer ^(devkitProUpdater-*.exe^) from
echo        https://github.com/devkitPro/installer/releases/latest
echo   2. Run it. Install to a folder named devkitPro at the top of any drive
echo      ^(C:\devkitPro, D:\devkitPro, ...^). When it asks what to install,
echo      tick "Wii Development" and untick the rest.
echo   3. Run build.bat again. It offers to install Rust too, if you don't have it.
echo.
echo   ^(Docker Desktop works too, if you already use it.^)
echo.
echo   No installs at all: on github.com open the repo, Actions, the newest "ci"
echo   run, and download "flatground-sd-card" at the bottom of the page.
goto failed

:done
echo.
if /i not "%TARGET%"=="package" (
    echo Cleaned.
    goto finish
)
if defined FG_SD call :copy_sd
echo Done. Copy this folder into the "apps" folder on the Wii's SD card:
echo   %cd%\dist\apps\flatground
echo ^(so the card has apps\flatground\boot.dol^), then launch it from the Homebrew Channel.
echo Converted packs from your own disc ^(if any^) are in its data folder.
if "%DOLPHIN%"=="1" call :run_dolphin
goto finish

rem ---- optional: start the game in Dolphin -------------------------------------
:run_dolphin
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0tools\dolphin.ps1"
exit /b 0

rem ---- optional: copy to the SD card ------------------------------------------
:copy_sd
set "SD=%FG_SD:"=%"
if not exist "%SD%\" (
    echo [sd] %SD% isn't there ^(card not inserted?^), so not copying.
    exit /b 0
)
rem robocopy copies only new or changed files; exit codes under 8 mean success.
robocopy "dist\apps\flatground" "%SD%\apps\flatground" /e /njh /njs /ndl /nfl /np >nul
if errorlevel 8 (
    echo [sd] Copying to %SD% failed.
) else (
    echo [sd] Copied to %SD%\apps\flatground
)
exit /b 0

:finish
echo.
if "%PAUSE%"=="1" pause
endlocal
exit /b 0

:failed
echo.
echo Build failed. See the messages above.
echo.
if "%PAUSE%"=="1" pause
endlocal
exit /b 1
