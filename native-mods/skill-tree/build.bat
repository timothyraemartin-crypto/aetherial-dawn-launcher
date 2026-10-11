@echo off
rem Usage: build.bat <source.cpp> <output.dll>  (uses local include\adlite.h with a shareable log, then the shared kit)
call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1
set TC=A:\Aetherial Dawn\skse-toolchain\
if not exist %TEMP%\adlite2 mkdir %TEMP%\adlite2
cl /nologo /std:c++17 /O2 /EHsc /MT /W3 /LD /I "%~dp0include" /I "%TC%minhook\include" ^
  "%~1" "%TC%minhook\src\buffer.c" "%TC%minhook\src\hook.c" "%TC%minhook\src\trampoline.c" "%TC%minhook\src\hde\hde64.c" ^
  /Fe"%~2" /Fo%TEMP%\adlite2\ /link /DLL shell32.lib user32.lib
