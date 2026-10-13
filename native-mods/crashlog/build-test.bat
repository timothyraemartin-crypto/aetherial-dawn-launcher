@echo off
rem Usage: build-test.bat <toolchain-dir> <out.exe>   (toolchain-dir = the SKSE kit with include\ and minhook\)
if "%~2"=="" (echo usage: build-test.bat toolchain-dir out.exe & exit /b 2)
call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1
cl /nologo /std:c++17 /O2 /EHsc /MT /W3 /I "%~1\include" /I "%~1\minhook\include" "%~dp0tests\handler_test.cpp" ^
  "%~1\minhook\src\buffer.c" "%~1\minhook\src\hook.c" "%~1\minhook\src\trampoline.c" "%~1\minhook\src\hde\hde64.c" ^
  /Fe"%~2" /Fo%TEMP%\adlite\ /link shell32.lib user32.lib
