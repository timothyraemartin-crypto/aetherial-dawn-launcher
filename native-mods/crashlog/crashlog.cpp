// AetherialDawn-CrashLog 1.0.0 - writes a short crash report when Skyrim hits a fatal exception.
//
// Report file: Documents\My Games\Skyrim Special Edition\SKSE\AetherialDawn-Crash.log (last run: .prev.log)
// Plugin log:  ...\SKSE\AetherialDawn-CrashLog.log (also gets the report, line by line)
// The launcher adds AetherialDawn-Crash.log to the crash report it sends to staff.
//
// It never hooks or changes the game: a vectored exception handler (last in line) and an
// unhandled-exception filter write the report and pass the exception on. See handler.h.
//
// Kill switch: create Data\SKSE\Plugins\AetherialDawn-Crash.off (any content) - the plugin then installs nothing.
//
// The open menus in the report are read (never written) from the engine's UI object, with the same
// Address Library ids and layout AetherialDawn-NoPause verified on 1.6.1170; if anything is off the line
// says so and the rest of the report is unaffected.
#include "handler.h"

namespace {

constexpr const char* kVersion = "1.0.0";
constexpr uint64_t kUiSingletonId = 400327;
constexpr size_t kMenuStack = 0x110, kNumPauses = 0x160, kFlags = 0x1C;

struct MenuName { const char* name; uint64_t vtableId; uintptr_t vtable; };
MenuName g_menus[] = {
    {"Journal", 216685, 0}, {"Inventory", 215494, 0}, {"Magic", 215652, 0}, {"Favorites", 215307, 0},
    {"Map", 216502, 0}, {"Container", 215061, 0}, {"Crafting", 215111, 0}, {"Barter", 214926, 0},
    {"Gift", 215323, 0}, {"Book", 214975, 0}, {"Stats", 215973, 0}, {"Tween", 216145, 0},
    {"SleepWait", 215936, 0}, {"Training", 216116, 0}, {"LevelUp", 215560, 0}, {"MessageBox", 215795, 0},
    {"Lockpicking", 215604, 0}, {"Tutorial", 216133, 0}, {"Console", 215002, 0},
};
uintptr_t g_uiPtrAddr = 0;
bool g_haveMenus = false;

void AppendTo(char* out, size_t n, const char* fmt, ...) {
    const size_t used = strlen(out);
    if (used + 1 >= n) return;
    va_list a; va_start(a, fmt);
    _vsnprintf_s(out + used, n - used, _TRUNCATE, fmt, a);
    va_end(a);
}

// Runs on the crash report thread, under SEH (handler.h).
void ActiveMenus(char* out, size_t n) {
    out[0] = 0;
    if (!g_haveMenus) { strcpy_s(out, n, "(unknown: Address Library not available)"); return; }
    uint8_t* ui = *reinterpret_cast<uint8_t**>(g_uiPtrAddr);
    if (!ui || !adlite::Readable(ui + kMenuStack, 0x18)) { strcpy_s(out, n, "(UI not up yet)"); return; }
    uint8_t** data = *reinterpret_cast<uint8_t***>(ui + kMenuStack);
    const uint32_t size = *reinterpret_cast<uint32_t*>(ui + kMenuStack + 0x10);
    const uint32_t cap = *reinterpret_cast<uint32_t*>(ui + kMenuStack + 0x08);
    if (size == 0 || !data) { strcpy_s(out, n, "(none open)"); return; }
    if (size > 32 || cap < size || !adlite::Readable(data, size * 8)) { _snprintf_s(out, n, _TRUNCATE, "(menu stack unreadable: size=%u cap=%u)", size, cap); return; }
    for (uint32_t i = 0; i < size; ++i) {
        uint8_t* menu = data[i];
        if (!menu || !adlite::Readable(menu, 0x30)) { AppendTo(out, n, "%s(null)", i ? ", " : ""); continue; }
        const uintptr_t vt = *reinterpret_cast<uintptr_t*>(menu);
        const char* name = nullptr;
        for (const auto& m : g_menus) if (m.vtable && m.vtable == vt) name = m.name;
        if (name) AppendTo(out, n, "%s%s", i ? ", " : "", name);
        else AppendTo(out, n, "%sother(vtable base+0x%llx)", i ? ", " : "", static_cast<unsigned long long>(vt - adlite::Base()));
    }
    AppendTo(out, n, "  [numPausesGame=%u]", *reinterpret_cast<uint32_t*>(ui + kNumPauses));
}

bool Load(const void*) {
    adlite::Log::Open("AetherialDawn-CrashLog");
    adlite::Log::Write("AetherialDawn-CrashLog %s loading", kVersion);
    wchar_t exe[MAX_PATH]{};
    GetModuleFileNameW(nullptr, exe, MAX_PATH);
    std::wstring dir(exe);
    dir = dir.substr(0, dir.find_last_of(L"\\/"));
    if (GetFileAttributesW((dir + L"\\Data\\SKSE\\Plugins\\AetherialDawn-Crash.off").c_str()) != INVALID_FILE_ATTRIBUTES) {
        adlite::Log::Write("AetherialDawn-Crash.off found; crash handler not installed");
        return true;
    }
    adlite::AddrLib lib;
    if (lib.Load()) {
        g_uiPtrAddr = lib.Address(kUiSingletonId);
        for (auto& m : g_menus) m.vtable = lib.Address(m.vtableId);
        g_haveMenus = g_uiPtrAddr != 0;
    }
    adlite::Log::Write("open-menu names in the report: %s", g_haveMenus ? "on" : "off (Address Library missing)");
    if (!crashlog::Install(kVersion, ActiveMenus, nullptr)) {
        adlite::Log::Write("crash handler could not be installed");
        return true;
    }
    adlite::Log::Write("crash handler installed (vectored handler last in line + unhandled-exception filter); report file AetherialDawn-Crash.log");
    return true;  // never block the game
}

}  // namespace

ADLITE_PLUGIN("AetherialDawn-CrashLog", "Aetherial Dawn", 0x01000000, Load)
