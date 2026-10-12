#pragma once
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>
#include <algorithm>
#include <atomic>
#include <map>
#include <set>
#include <unordered_map>
#include <strings.h>
typedef uint32_t DWORD; typedef void* LPVOID;
#define WINAPI
#define MAX_PATH 260
#define PAGE_READWRITE 4
#define INVALID_FILE_ATTRIBUTES 0xFFFFFFFF
#undef __try
#define __try if (true)
#define __except(x) else
#define EXCEPTION_EXECUTE_HANDLER 1
inline bool VirtualProtect(void*, size_t, DWORD, DWORD*) { return true; }
inline void Sleep(DWORD) {}
inline void* CreateThread(void*, size_t, DWORD (*)(LPVOID), LPVOID, DWORD, DWORD*) { return nullptr; }
inline DWORD GetModuleFileNameW(void*, wchar_t*, DWORD) { return 0; }
inline DWORD GetFileAttributesW(const wchar_t*) { return 0; }
inline int _wfopen_s(FILE**, const wchar_t*, const wchar_t*) { return 1; }
inline int _strnicmp(const char* a, const char* b, size_t n) { return strncasecmp(a, b, n); }
#include <strings.h>
namespace adlite {
struct AddrLib { bool Load() { return true; } uintptr_t Address(uint64_t) { return 0; } };
struct Log { static void Open(const char*) {} static void Write(const char*, ...) {} };
inline bool InitHooks() { return true; }
inline bool Hook(uintptr_t, void*, void**) { return true; }
inline bool Readable(const void*, size_t) { return true; }
}
#define ADLITE_PLUGIN(NAME, AUTHOR, VERSION, LOADFN)
