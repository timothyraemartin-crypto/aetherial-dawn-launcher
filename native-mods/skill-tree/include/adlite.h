// adlite.h - tiny dependency-free helpers for Aetherial Dawn SKSE plugins.
// Targets Skyrim SE/AE 1.6.1170 with SKSE 2.2.6. No CommonLibSSE, no SKSE headers.
//   - ADLITE_PLUGIN(name, author, version, load_fn): exports SKSEPlugin_Version + SKSEPlugin_Load
//   - adlite::Log: line logger to Documents\My Games\Skyrim Special Edition\SKSE\<name>.log
//   - adlite::AddrLib: reads Data\SKSE\Plugins\versionlib-1-6-1170-0.bin (Address Library ids -> offsets)
//   - adlite::Hook: MinHook detour wrapper by address-library id
#pragma once
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <shlobj.h>
#include <share.h>
#include <algorithm>
#include <cstdarg>
#include <cstdint>
#include <cstdio>
#include <string>
#include <vector>
#include "MinHook.h"

namespace adlite {

inline uintptr_t Base() {
    static uintptr_t b = reinterpret_cast<uintptr_t>(GetModuleHandleW(nullptr));
    return b;
}

// ---------------------------------------------------------------- logging
class Log {
public:
    static void Open(const char* name) {
        wchar_t docs[MAX_PATH]{};
        if (SUCCEEDED(SHGetFolderPathW(nullptr, CSIDL_PERSONAL, nullptr, 0, docs))) {
            std::wstring dir = std::wstring(docs) + L"\\My Games\\Skyrim Special Edition\\SKSE";
            CreateDirectoryW(dir.c_str(), nullptr);
            std::wstring file = dir + L"\\" + std::wstring(name, name + strlen(name)) + L".log";
            F() = _wfsopen(file.c_str(), L"w", _SH_DENYNO);
        }
    }
    static void Write(const char* fmt, ...) {
        FILE* f = F();
#ifdef ADLITE_STDERR
        if (!f) f = stderr;
#endif
        if (!f) return;
        SYSTEMTIME t; GetLocalTime(&t);
        fprintf(f, "[%02d:%02d:%02d.%03d] ", t.wHour, t.wMinute, t.wSecond, t.wMilliseconds);
        va_list a; va_start(a, fmt); vfprintf(f, fmt, a); va_end(a);
        fputc('\n', f);
        fflush(f);
    }
private:
    static FILE*& F() { static FILE* f = nullptr; return f; }
};

// ---------------------------------------------------------------- address library
class AddrLib {
public:
    // Loads the 1.6.1170 database. Returns false (and logs why) on any problem.
    bool Load() {
        wchar_t exe[MAX_PATH]{};
        GetModuleFileNameW(nullptr, exe, MAX_PATH);
        std::wstring dir(exe);
        dir = dir.substr(0, dir.find_last_of(L"\\/"));
        return LoadFile(dir + L"\\Data\\SKSE\\Plugins\\versionlib-1-6-1170-0.bin");
    }
    bool LoadFile(const std::wstring& path) {
        FILE* f = nullptr;
        if (_wfopen_s(&f, path.c_str(), L"rb") != 0 || !f) { Log::Write("addrlib: cannot open versionlib-1-6-1170-0.bin"); return false; }
        std::vector<uint8_t> buf;
        fseek(f, 0, SEEK_END); long n = ftell(f); fseek(f, 0, SEEK_SET);
        buf.resize(static_cast<size_t>(n));
        fread(buf.data(), 1, buf.size(), f);
        fclose(f);
        size_t p = 0;
        auto rd = [&](void* dst, size_t sz) -> bool {
            if (p + sz > buf.size()) return false;
            memcpy(dst, &buf[p], sz); p += sz; return true;
        };
        int32_t format = 0, ver[4]{}, nameLen = 0, ptrSize = 0, count = 0;
        if (!rd(&format, 4) || !rd(ver, 16) || !rd(&nameLen, 4)) return false;
        p += static_cast<size_t>(nameLen);
        if (!rd(&ptrSize, 4) || !rd(&count, 4)) return false;
        if (format != 2 || ver[0] != 1 || ver[1] != 6 || ver[2] != 1170 || ptrSize <= 0 || count <= 0) {
            Log::Write("addrlib: unexpected header format=%d ver=%d.%d.%d.%d ptr=%d count=%d", format, ver[0], ver[1], ver[2], ver[3], ptrSize, count);
            return false;
        }
        map_.reserve(static_cast<size_t>(count));
        uint64_t id = 0, off = 0, pid = 0, poff = 0;
        for (int i = 0; i < count; ++i) {
            uint8_t type = 0;
            if (!rd(&type, 1)) return false;
            const uint8_t lo = type & 0xF, hi = type >> 4;
            uint8_t b8 = 0; uint16_t b16 = 0; uint32_t b32 = 0;
            switch (lo) {
                case 0: if (!rd(&id, 8)) return false; break;
                case 1: id = pid + 1; break;
                case 2: if (!rd(&b8, 1)) return false; id = pid + b8; break;
                case 3: if (!rd(&b8, 1)) return false; id = pid - b8; break;
                case 4: if (!rd(&b16, 2)) return false; id = pid + b16; break;
                case 5: if (!rd(&b16, 2)) return false; id = pid - b16; break;
                case 6: if (!rd(&b16, 2)) return false; id = b16; break;
                case 7: if (!rd(&b32, 4)) return false; id = b32; break;
                default: return false;
            }
            const uint64_t tmp = (hi & 8) ? (poff / static_cast<uint64_t>(ptrSize)) : poff;
            switch (hi & 7) {
                case 0: if (!rd(&off, 8)) return false; break;
                case 1: off = tmp + 1; break;
                case 2: if (!rd(&b8, 1)) return false; off = tmp + b8; break;
                case 3: if (!rd(&b8, 1)) return false; off = tmp - b8; break;
                case 4: if (!rd(&b16, 2)) return false; off = tmp + b16; break;
                case 5: if (!rd(&b16, 2)) return false; off = tmp - b16; break;
                case 6: if (!rd(&b16, 2)) return false; off = b16; break;
                case 7: if (!rd(&b32, 4)) return false; off = b32; break;
            }
            if (hi & 8) off *= static_cast<uint64_t>(ptrSize);
            map_.emplace_back(id, off);
            poff = off; pid = id;
        }
        std::sort(map_.begin(), map_.end());
        Log::Write("addrlib: loaded %zu ids (1.6.1170)", map_.size());
        return true;
    }
    // Offset from the image base, or 0 if unknown.
    uint64_t Offset(uint64_t id) const {
        auto it = std::lower_bound(map_.begin(), map_.end(), std::make_pair(id, uint64_t(0)));
        return (it != map_.end() && it->first == id) ? it->second : 0;
    }
    uintptr_t Address(uint64_t id) const {
        auto o = Offset(id);
        return o ? Base() + static_cast<uintptr_t>(o) : 0;
    }
private:
    std::vector<std::pair<uint64_t, uint64_t>> map_;
};

// ---------------------------------------------------------------- hooking
inline bool InitHooks() { return MH_Initialize() == MH_OK; }

// Detours `target` to `detour`; *original receives the callable trampoline.
inline bool Hook(uintptr_t target, void* detour, void** original) {
    if (!target) return false;
    if (MH_CreateHook(reinterpret_cast<void*>(target), detour, original) != MH_OK) return false;
    return MH_EnableHook(reinterpret_cast<void*>(target)) == MH_OK;
}

// Safe read helpers (return false instead of faulting).
inline bool Readable(const void* p, size_t n) {
    MEMORY_BASIC_INFORMATION mbi{};
    if (!VirtualQuery(p, &mbi, sizeof(mbi))) return false;
    if (mbi.State != MEM_COMMIT || (mbi.Protect & (PAGE_NOACCESS | PAGE_GUARD))) return false;
    return reinterpret_cast<uintptr_t>(p) + n <= reinterpret_cast<uintptr_t>(mbi.BaseAddress) + mbi.RegionSize;
}

}  // namespace adlite

// ---------------------------------------------------------------- SKSE 2.2.6 plugin exports
struct ADSKSEPluginVersionData {
    uint32_t dataVersion;
    uint32_t pluginVersion;
    char name[256];
    char author[256];
    char supportEmail[252];
    uint32_t versionIndependenceEx;  // bit0: no struct use
    uint32_t versionIndependence;
    uint32_t compatibleVersions[16];
    uint32_t seVersionRequired;
};
static_assert(sizeof(ADSKSEPluginVersionData) == 0x350, "SKSE version data layout");

// Packed Skyrim runtime 1.6.1170.0
#define AD_RUNTIME_1_6_1170 ((1u << 24) | (6u << 16) | (1170u << 4))

#define ADLITE_PLUGIN(NAME, AUTHOR, VERSION, LOADFN)                                           \
    extern "C" __declspec(dllexport) ADSKSEPluginVersionData SKSEPlugin_Version = {            \
        1, (VERSION), NAME, AUTHOR, "", 1u, 0u, {AD_RUNTIME_1_6_1170}, 0};                     \
    extern "C" __declspec(dllexport) bool SKSEPlugin_Load(const void* skse) { return LOADFN(skse); }
