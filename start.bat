@echo off
setlocal enabledelayedexpansion
title YouTube Summarizer

set "SCRIPT_DIR=%~dp0"
cd /d "%SCRIPT_DIR%"

:: ── Python-Check ─────────────────────────────────────────────
where python >nul 2>&1
if %errorlevel% neq 0 (
    echo Python ist nicht installiert.
    echo Bitte installiere Python 3.9+ von https://python.org
    echo Wichtig: Bei der Installation "Add Python to PATH" aktivieren!
    pause
    exit /b 1
)

for /f "tokens=2 delims= " %%v in ('python -c "import sys; print(sys.version.split()[0])"') do set "PY_VER=%%v"
echo Python %PY_VER%

:: ── Abhangigkeiten prufen ────────────────────────────────────
set "MISSING="
set "PKG_LIST=PySide6 youtube_transcript_api sqlalchemy httpx"

for %%p in (%PKG_LIST%) do (
    python -c "import %%p" >nul 2>&1
    if !errorlevel! neq 0 (
        echo [FEHLT] %%p
        set "MISSING=!MISSING! %%p"
    ) else (
        echo [OK]     %%p
    )
)

:: ── Konfiguration ────────────────────────────────────────────
if not exist "config.json" (
    if exist "config.example.json" (
        copy config.example.json config.json >nul
        echo config.example.json -^> config.json kopiert.
        echo Bitte API-Key in config.json eintragen!
    )
)

:: ── Installation anfragen ────────────────────────────────────
if not "%MISSING%"=="" (
    echo.
    echo Fehlende Pakete: %MISSING%
    set /p "INSTALL=Jetzt installieren? [Y/n] "
    if /i not "!INSTALL!"=="n" (
        echo.
        echo Installiere...
        pip install --break-system-packages %MISSING%
        if !errorlevel! neq 0 (
            echo.
            echo Installation fehlgeschlagen. Versuche mit --user...
            pip install --user %MISSING%
        )
        echo.
        echo Fertig.
    )
)

:: ── Start ────────────────────────────────────────────────────
echo.
echo Starte YouTube Summarizer...
echo.
python main.py
if %errorlevel% neq 0 pause
