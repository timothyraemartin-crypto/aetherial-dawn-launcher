// AetherialQuietPickups: hides Skyrim's vanilla top-left "item added / removed" HUD notices.
// Aetherial Dawn's own CEF/F3 GUI shows item gains, so the vanilla ones are redundant.
// Event-driven only: one detour on HUDMenu::ProcessMessage. No per-tick work, no inventory/equipment calls.
// Offsets/layouts from CommonLibSSE-NG (AE ids): VTABLE_HUDMenu[0]=215362, VTABLE_HUDData=216313.
#include "adlite.h"
#include <atomic>
#include <cctype>
#include <string>
#include <vector>

namespace {
constexpr uint64_t kHudMenuVtableId = 215362;  // HUDMenu : IMenu primary vtable (AE)
constexpr uint64_t kHudDataVtableId = 216313;  // HUDData : IUIMessageData (AE)
constexpr size_t   kProcessMessageSlot = 4;
constexpr uint32_t kMsgUpdate = 0;           // UI_MESSAGE_TYPE::kUpdate
constexpr uint32_t kHudNotification = 1;     // HUDData::Type::kNotification
constexpr uint32_t kHandled = 0;             // UI_MESSAGE_RESULTS::kHandled

std::atomic<bool> g_enabled{true};
std::atomic<bool> g_log{false};
std::vector<std::string> g_needles{" added", " removed"};  // lower-case, matched case-insensitively
uintptr_t g_hudDataVtbl = 0;
uint32_t (*g_orig)(void*, void*) = nullptr;

std::string Lower(std::string s) { for (auto& c : s) c = (char)std::tolower((unsigned char)c); return s; }

bool Hide(const std::string& text) {
    auto l = Lower(text);
    for (auto& n : g_needles) if (l.find(n) != std::string::npos) return true;
    return false;
}

// POD-only SEH probe: true if msg is a HUDData kNotification; copies its text into buf.
bool ReadNotification(void* msg, char* buf, size_t n) {
    __try {
        auto* m = static_cast<uint8_t*>(msg);
        if (!m || *reinterpret_cast<uint32_t*>(m + 0x08) != kMsgUpdate) return false;
        auto* d = *reinterpret_cast<uint8_t**>(m + 0x10);
        if (!d || *reinterpret_cast<uintptr_t*>(d) != g_hudDataVtbl) return false;
        if (*reinterpret_cast<uint32_t*>(d + 0x10) != kHudNotification) return false;
        const char* t = *reinterpret_cast<const char**>(d + 0x18);
        size_t i = 0;
        if (t) for (; i + 1 < n && t[i]; ++i) buf[i] = t[i];
        buf[i] = 0;
        return true;
    } __except (EXCEPTION_EXECUTE_HANDLER) {
        return false;  // layout surprise: fall through to vanilla, never crash
    }
}

uint32_t Detour(void* menu, void* msg) {
    // UIMessage: type 08, data 10. HUDData: vptr 00, type 10, BSString text 18 (char* first).
    char buf[512];
    if (ReadNotification(msg, buf, sizeof buf)) {
        const bool hit = g_enabled.load() && Hide(buf);
        if (g_log.load()) adlite::Log::Write("HUD notification \"%s\" -> %s", buf, hit ? "HIDDEN" : "shown");
        if (hit) return kHandled;
    }
    return g_orig(menu, msg);
}

void LoadIni() {
    wchar_t exe[MAX_PATH]{};
    GetModuleFileNameW(nullptr, exe, MAX_PATH);
    std::wstring dir(exe);
    dir = dir.substr(0, dir.find_last_of(L"\\/")) + L"\\Data\\SKSE\\Plugins\\AetherialQuietPickups.ini";
    FILE* f = nullptr;
    if (_wfopen_s(&f, dir.c_str(), L"r") != 0 || !f) { adlite::Log::Write("ini not found, using defaults"); return; }
    char line[512];
    while (fgets(line, sizeof line, f)) {
        std::string s(line);
        while (!s.empty() && (s.back() == '\n' || s.back() == '\r' || s.back() == ' ')) s.pop_back();
        if (s.rfind("Enabled=", 0) == 0) g_enabled = s.substr(8) != "0";
        else if (s.rfind("LogNotifications=", 0) == 0) g_log = s.substr(17) == "1";
        else if (s.rfind("HideContains=", 0) == 0) {  // ';'-separated, kept verbatim (leading space matters)
            g_needles.clear();
            std::string v = s.substr(13), cur;
            for (char c : v + ";") { if (c == ';') { if (!cur.empty()) g_needles.push_back(Lower(cur)); cur.clear(); } else cur += c; }
        }
    }
    fclose(f);
}

bool Load(const void*) {
    adlite::Log::Open("AetherialQuietPickups");
    LoadIni();
    adlite::AddrLib lib;
    if (!lib.Load()) return true;  // never block game start
    const uintptr_t menuVtbl = lib.Address(kHudMenuVtableId);
    g_hudDataVtbl = lib.Address(kHudDataVtableId);
    if (!menuVtbl || !g_hudDataVtbl || !adlite::InitHooks()) { adlite::Log::Write("setup failed (ids/minhook)"); return true; }
    const uintptr_t target = reinterpret_cast<const uintptr_t*>(menuVtbl)[kProcessMessageSlot];
    const bool ok = adlite::Hook(target, reinterpret_cast<void*>(&Detour), reinterpret_cast<void**>(&g_orig));
    adlite::Log::Write("HUDMenu::ProcessMessage hook %s (target base+0x%llx) enabled=%d log=%d needles=%zu",
                       ok ? "installed" : "FAILED", (unsigned long long)(target - adlite::Base()), (int)g_enabled.load(), (int)g_log.load(), g_needles.size());
    return true;
}
}  // namespace

ADLITE_PLUGIN("AetherialQuietPickups", "Aetherial Dawn", 1, Load)
