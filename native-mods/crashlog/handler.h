// handler.h - the crash handler of AetherialDawn-CrashLog.
//
// Writes a short report when the game hits a fatal-looking exception, to
//   * AetherialDawn-Crash.log   (Documents\My Games\Skyrim Special Edition\SKSE\, last run kept as .prev.log)
//   * the plugin's own AetherialDawn-CrashLog.log (same text, line by line)
// Report: exception code and address as module+offset, a short stack walk, the active menus
// (callback), the loaded SKSE plugins, the other loaded DLLs.
//
// It never changes how the game behaves: the vectored handler (registered last) and the
// unhandled-exception filter only write the report; both then pass the exception on.
//   * VEH (first-chance): fatal-looking codes only, at most 3 reports, repeats of the same
//     code+address skipped. A handled exception is labelled "first-chance" (the game may carry on).
//   * UEF: the real crash; one report; the previous filter is called afterwards.
// The report is built on a worker thread made at load time (so a stack overflow or a crash in the
// loader does not need new stack or a new thread), the faulting thread waits at most 8 s for it.
// Kill switch: Data\SKSE\Plugins\AetherialDawn-Crash.off (checked by the caller before Install).
#pragma once
#include "adlite.h"
#include <psapi.h>

namespace crashlog {

constexpr size_t kBuf = 96 * 1024;
constexpr LONG kMaxFirstChance = 3;

using MenusFn = void (*)(char* out, size_t n);  // fills in the open menu names; runs under SEH on the worker thread

inline wchar_t g_path[MAX_PATH];
inline const char* g_version = "";
inline MenusFn g_menus = nullptr;
inline volatile LONG* g_skip = nullptr;  // non-zero while the plugin itself is inside a __try: skip first-chance reports
inline HANDLE g_evGo = nullptr, g_evDone = nullptr;
inline DWORD g_workerId = 0;
inline EXCEPTION_RECORD g_rec;
inline CONTEXT g_ctx;
inline DWORD g_faultTid = 0;
inline bool g_unhandled = false;
inline volatile LONG g_busy = 0, g_firstChance = 0, g_unhandledCount = 0;
inline uint64_t g_seenCode[8];
inline uintptr_t g_seenAddr[8];
inline int g_seenN = 0;
inline LPTOP_LEVEL_EXCEPTION_FILTER g_prevUef = nullptr;
inline ULONGLONG g_loadTick = 0;
inline char g_buf[kBuf];
inline size_t g_len = 0;

inline void Raw(const char* fmt, ...) {
    if (g_len >= kBuf - 1) return;
    va_list a; va_start(a, fmt);
    const int n = _vsnprintf_s(g_buf + g_len, kBuf - g_len, _TRUNCATE, fmt, a);
    va_end(a);
    g_len = n >= 0 ? g_len + static_cast<size_t>(n) : kBuf - 1;
}
inline void Out(const char* fmt, ...) {
    if (g_len >= kBuf - 2) return;
    va_list a; va_start(a, fmt);
    const int n = _vsnprintf_s(g_buf + g_len, kBuf - g_len - 1, _TRUNCATE, fmt, a);
    va_end(a);
    g_len = n >= 0 ? g_len + static_cast<size_t>(n) : kBuf - 2;
    g_buf[g_len++] = '\n';
    g_buf[g_len] = 0;
}

inline const char* CodeName(DWORD c) {
    switch (c) {
        case 0xC0000005: return "EXCEPTION_ACCESS_VIOLATION";
        case 0xC0000006: return "EXCEPTION_IN_PAGE_ERROR";
        case 0xC000001D: return "EXCEPTION_ILLEGAL_INSTRUCTION";
        case 0xC0000025: return "EXCEPTION_NONCONTINUABLE_EXCEPTION";
        case 0xC0000094: return "EXCEPTION_INT_DIVIDE_BY_ZERO";
        case 0xC0000095: return "EXCEPTION_INT_OVERFLOW";
        case 0xC0000096: return "EXCEPTION_PRIV_INSTRUCTION";
        case 0xC000008C: return "EXCEPTION_ARRAY_BOUNDS_EXCEEDED";
        case 0xC00000FD: return "EXCEPTION_STACK_OVERFLOW";
        case 0xC0000374: return "STATUS_HEAP_CORRUPTION";
        case 0xC0000409: return "STATUS_STACK_BUFFER_OVERRUN";
        case 0xE06D7363: return "C++ exception (uncaught)";
        default: return "exception";
    }
}

// Fatal-looking codes worth a first-chance report. C++ throws, debugger names and breakpoints are not.
inline bool Fatal(DWORD c) {
    switch (c) {
        case 0xC0000005: case 0xC0000006: case 0xC000001D: case 0xC0000025: case 0xC0000094:
        case 0xC0000095: case 0xC0000096: case 0xC000008C: case 0xC00000FD: case 0xC0000374:
        case 0xC0000409: return true;
        default: return false;
    }
}

// "SkyrimSE.exe+0x1234" (module file name + offset from its base), or the bare address.
inline void Where(uintptr_t a, char* out, size_t n) {
    MEMORY_BASIC_INFORMATION mbi{};
    if (VirtualQuery(reinterpret_cast<void*>(a), &mbi, sizeof(mbi)) && mbi.AllocationBase && mbi.Type == MEM_IMAGE) {
        wchar_t path[MAX_PATH]{};
        if (K32GetModuleFileNameExW(GetCurrentProcess(), static_cast<HMODULE>(mbi.AllocationBase), path, MAX_PATH)) {
            const wchar_t* base = wcsrchr(path, L'\\');
            _snprintf_s(out, n, _TRUNCATE, "%ls+0x%llx", base ? base + 1 : path, static_cast<unsigned long long>(a - reinterpret_cast<uintptr_t>(mbi.AllocationBase)));
            return;
        }
    }
    _snprintf_s(out, n, _TRUNCATE, "0x%llx (no module)", static_cast<unsigned long long>(a));
}

// Walk up the stack from the fault's CONTEXT with the x64 unwind tables (the handler's own frames are not in it).
inline int Walk(CONTEXT ctx, uintptr_t* out, int max) {
    int n = 0;
    __try {
        while (n < max && ctx.Rip) {
            out[n++] = static_cast<uintptr_t>(ctx.Rip);
            DWORD64 imageBase = 0;
            RUNTIME_FUNCTION* rf = RtlLookupFunctionEntry(ctx.Rip, &imageBase, nullptr);
            if (!rf) {  // leaf or generated code: assume the return address is on top of the stack
                if (!adlite::Readable(reinterpret_cast<void*>(ctx.Rsp), 8)) break;
                ctx.Rip = *reinterpret_cast<DWORD64*>(ctx.Rsp);
                ctx.Rsp += 8;
            } else {
                void* handlerData = nullptr; DWORD64 frame = 0;
                RtlVirtualUnwind(UNW_FLAG_NHANDLER, imageBase, ctx.Rip, rf, &ctx, &handlerData, &frame, nullptr);
            }
        }
    } __except (EXCEPTION_EXECUTE_HANDLER) {}
    return n;
}

inline void ListModules() {
    static HMODULE mods[1024];
    DWORD need = 0;
    if (!K32EnumProcessModules(GetCurrentProcess(), mods, sizeof(mods), &need)) { Out("(module list unavailable)"); return; }
    const size_t n = (std::min)(static_cast<size_t>(need) / sizeof(HMODULE), static_cast<size_t>(1024));
    static wchar_t path[MAX_PATH], low[MAX_PATH];
    unsigned plugins = 0, system = 0, others = 0;
    Out("SKSE PLUGINS (Data\\SKSE\\Plugins):");
    for (size_t i = 0; i < n; ++i) {
        if (!K32GetModuleFileNameExW(GetCurrentProcess(), mods[i], path, MAX_PATH)) continue;
        wcscpy_s(low, path); _wcslwr_s(low);
        if (!wcsstr(low, L"\\data\\skse\\plugins\\")) continue;
        MODULEINFO mi{};
        K32GetModuleInformation(GetCurrentProcess(), mods[i], &mi, sizeof(mi));
        const wchar_t* b = wcsrchr(path, L'\\');
        Out("  %ls  base=0x%llx size=0x%x", b ? b + 1 : path, static_cast<unsigned long long>(reinterpret_cast<uintptr_t>(mods[i])), static_cast<unsigned>(mi.SizeOfImage));
        ++plugins;
    }
    if (!plugins) Out("  (none)");
    Out("OTHER DLLS (names only; Windows system DLLs counted):");
    Raw("  ");
    for (size_t i = 0; i < n; ++i) {
        if (!K32GetModuleFileNameExW(GetCurrentProcess(), mods[i], path, MAX_PATH)) continue;
        wcscpy_s(low, path); _wcslwr_s(low);
        if (wcsstr(low, L"\\data\\skse\\plugins\\")) continue;
        if (wcsstr(low, L"\\windows\\")) { ++system; continue; }
        const wchar_t* b = wcsrchr(path, L'\\');
        Raw("%s%ls", others ? ", " : "", b ? b + 1 : path);
        ++others;
    }
    Out("");
    Out("  (%u other, %u Windows system, %u SKSE plugins, %zu modules in all)", others, system, plugins, n);
}

inline void BuildReport() {
    g_len = 0; g_buf[0] = 0;
    const DWORD code = g_rec.ExceptionCode;
    char where[300];
    Where(reinterpret_cast<uintptr_t>(g_rec.ExceptionAddress), where, sizeof(where));
    SYSTEMTIME t; GetLocalTime(&t);
    Out("===== AetherialDawn-CrashLog %s crash report %04d-%02d-%02d %02d:%02d:%02d =====", g_version, t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond);
    Out("Exception 0x%08lx %s at %s [%s]", code, CodeName(code), where,
        g_unhandled ? "UNHANDLED: the game is going down" : "FIRST-CHANCE: the game may handle this and carry on");
    if (code == 0xC0000005 && g_rec.NumberParameters >= 2) {
        const ULONG_PTR kind = g_rec.ExceptionInformation[0];
        Out("  %s address 0x%llx", kind == 0 ? "reading" : kind == 1 ? "writing" : kind == 8 ? "executing (DEP)" : "touching", static_cast<unsigned long long>(g_rec.ExceptionInformation[1]));
    }
    Out("Thread %lu, %llu s after the plugin loaded, report #%ld", g_faultTid, static_cast<unsigned long long>((GetTickCount64() - g_loadTick) / 1000), g_firstChance + g_unhandledCount);
    Out("CALL STACK:");
    uintptr_t frames[32];
    int n = Walk(g_ctx, frames, 24);
    if (n < 2) {  // no usable unwind data: fall back to the plain stack walk of this (worker) thread
        void* f2[24];
        const USHORT m = CaptureStackBackTrace(0, 24, f2, nullptr);
        Out("  (unwind from the fault failed; CaptureStackBackTrace of the report thread follows)");
        for (USHORT i = 0; i < m; ++i) frames[i] = reinterpret_cast<uintptr_t>(f2[i]);
        n = m;
    }
    for (int i = 0; i < n; ++i) {
        Where(frames[i], where, sizeof(where));
        Out("  [%d] %s", i, where);
    }
    char menus[600] = "(not available)";
    if (g_menus) {
        __try { g_menus(menus, sizeof(menus)); } __except (EXCEPTION_EXECUTE_HANDLER) { strcpy_s(menus, "(unreadable)"); }
    }
    Out("ACTIVE MENUS: %s", menus);
    ListModules();
    Out("===== end of report =====");
}

inline void WriteReport() {
    if (g_path[0]) {
        HANDLE h = CreateFileW(g_path, FILE_APPEND_DATA, FILE_SHARE_READ | FILE_SHARE_WRITE, nullptr, OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
        if (h != INVALID_HANDLE_VALUE) {
            DWORD w = 0;
            WriteFile(h, g_buf, static_cast<DWORD>(g_len), &w, nullptr);
            FlushFileBuffers(h);
            CloseHandle(h);
        }
    }
    // The same text in the plugin log, one log line per report line (last: it can block if the crash was inside the log).
    char* p = g_buf;
    while (*p) {
        char* e = strchr(p, '\n');
        if (e) *e = 0;
        adlite::Log::Write("%s", p);
        if (!e) break;
        p = e + 1;
    }
}

inline DWORD WINAPI Worker(LPVOID) {
    for (;;) {
        WaitForSingleObject(g_evGo, INFINITE);
        __try { BuildReport(); WriteReport(); } __except (EXCEPTION_EXECUTE_HANDLER) {}
        SetEvent(g_evDone);
    }
    return 0;
}

inline void Handle(EXCEPTION_POINTERS* ep, bool unhandled) {
    if (!g_evGo || GetCurrentThreadId() == g_workerId || !ep || !ep->ExceptionRecord || !ep->ContextRecord) return;
    if (!unhandled && g_skip && *g_skip) return;  // the plugin's own guarded read; it handles that itself
    if (InterlockedCompareExchange(&g_busy, 1, 0) != 0) return;
    const uintptr_t addr = reinterpret_cast<uintptr_t>(ep->ExceptionRecord->ExceptionAddress);
    bool skip = false;
    if (unhandled) {
        skip = InterlockedIncrement(&g_unhandledCount) > 1;
    } else {
        for (int i = 0; i < g_seenN; ++i) if (g_seenCode[i] == ep->ExceptionRecord->ExceptionCode && g_seenAddr[i] == addr) skip = true;
        if (!skip) {
            if (InterlockedIncrement(&g_firstChance) > kMaxFirstChance) skip = true;
            else if (g_seenN < 8) { g_seenCode[g_seenN] = ep->ExceptionRecord->ExceptionCode; g_seenAddr[g_seenN++] = addr; }
        }
    }
    if (skip) { InterlockedExchange(&g_busy, 0); return; }
    g_rec = *ep->ExceptionRecord;
    g_ctx = *ep->ContextRecord;
    g_faultTid = GetCurrentThreadId();
    g_unhandled = unhandled;
    ResetEvent(g_evDone);
    SetEvent(g_evGo);
    if (WaitForSingleObject(g_evDone, 8000) == WAIT_OBJECT_0) InterlockedExchange(&g_busy, 0);  // on a timeout stay busy: the worker is stuck
}

inline LONG CALLBACK Veh(EXCEPTION_POINTERS* ep) {
    if (ep && ep->ExceptionRecord && Fatal(ep->ExceptionRecord->ExceptionCode)) Handle(ep, false);
    return EXCEPTION_CONTINUE_SEARCH;
}

inline LONG WINAPI Uef(EXCEPTION_POINTERS* ep) {
    Handle(ep, true);
    return g_prevUef ? g_prevUef(ep) : EXCEPTION_CONTINUE_SEARCH;
}

// `dirOverride` is for the offline test only (the report goes there instead of Documents\...\SKSE).
inline bool Install(const char* version, MenusFn menus, volatile LONG* skip, const wchar_t* dirOverride = nullptr) {
    g_version = version; g_menus = menus; g_skip = skip; g_loadTick = GetTickCount64();
    std::wstring dir;
    if (dirOverride) {
        dir = dirOverride;
    } else {
        wchar_t docs[MAX_PATH]{};
        if (FAILED(SHGetFolderPathW(nullptr, CSIDL_PERSONAL, nullptr, 0, docs))) return false;
        dir = std::wstring(docs) + L"\\My Games\\Skyrim Special Edition\\SKSE";
        CreateDirectoryW(dir.c_str(), nullptr);
    }
    const std::wstring file = dir + L"\\AetherialDawn-Crash.log", prev = dir + L"\\AetherialDawn-Crash.prev.log";
    if (file.size() >= MAX_PATH) return false;
    MoveFileExW(file.c_str(), prev.c_str(), MOVEFILE_REPLACE_EXISTING);  // a crash report only ever exists for the run that wrote it
    wcscpy_s(g_path, file.c_str());
    g_evGo = CreateEventW(nullptr, FALSE, FALSE, nullptr);
    g_evDone = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    HANDLE th = g_evGo && g_evDone ? CreateThread(nullptr, 256 * 1024, Worker, nullptr, 0, &g_workerId) : nullptr;
    if (!th) { g_evGo = nullptr; return false; }
    CloseHandle(th);
    AddVectoredExceptionHandler(0, Veh);  // 0 = last in line
    g_prevUef = SetUnhandledExceptionFilter(Uef);
    return true;
}

}  // namespace crashlog
