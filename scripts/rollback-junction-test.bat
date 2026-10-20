@echo off
chcp 65001 >nul
setlocal
python "%~dp0rollback-junction-test.py" %*
set RC=%ERRORLEVEL%
echo.
echo ---- exit code: %RC% ----
pause
