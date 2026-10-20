@echo off
chcp 65001 >nul
setlocal
set PYTHONIOENCODING=utf-8
echo.
echo   CodeBuddy CN 会话共享迁移
echo   --------------------------
echo   1 = DRY-RUN（只统计，不改任何文件）
echo   2 = APPLY（真正执行：合并独有会话，删除副本，建 junction）
echo.
echo   提示：统计阶段要遍历 85 GB 级目录，可能持续几分钟。
echo         期间会不断刷新一行进度（文件数 / 已扫 GB / 速度），
echo         数字在跳就说明没卡死。Ctrl+C 可随时中断。
echo.
set /p choice=请输入 1 或 2 后回车:
echo.
if "%choice%"=="2" (
  python -u "%~dp0migrate-shared-sessions.py" --apply
) else (
  python -u "%~dp0migrate-shared-sessions.py"
)
set RC=%ERRORLEVEL%
echo.
echo ---- exit code: %RC% ----
pause
