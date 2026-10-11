// AetherialDawn-NoPause - keeps the game world running while selected menus are open.
//
// How it works (Skyrim SE/AE 1.6.1170):
//   * RE::UI keeps numPausesGame (ui+0x160). The engine adds 1 when a menu with the
//     kPausesGame flag (menu+0x1C bit 0) opens and subtracts 1 when it closes.
//     The world only advances while the counter is 0.
//   * Every frame, right after UIMessageQueue::ProcessCommands (which opens/closes menus),
//     we look at the open menu stack. For menus on our allow-list we clear kPausesGame,
//     and we rewrite numPausesGame to the number of open menus that still have the flag.
//     Because the flag is cleared, the engine's close path skips the decrement, so the
//     counter stays consistent and can never underflow.
//   * Menus not on the list (main menu, loading, race menu, fader, credits ...) are untouched.
//
// Layout (from CommonLibSSE-NG, verified against the game at runtime by checks below):
//   UI+0x110 BSTArray<IMenu*> menuStack {data@0, capacity@8, size@0x10}
//   UI+0x160 uint32 numPausesGame
//   IMenu+0x00 vtable, IMenu+0x1C uint32 menuFlags
//
// Optional Data\SKSE\Plugins\AetherialDawn-NoPause.ini:  [Menus] Journal=1 Inventory=1 ...  (0 to keep a menu paused)
#include "adlite.h"

namespace {

using ProcessCommands_t = void (*)(void*);
ProcessCommands_t g_orig = nullptr;

adlite::AddrLib g_lib;
uintptr_t g_uiPtrAddr = 0;  // address of the UI* singleton variable

constexpr uint64_t kUiSingletonId = 400327;
constexpr uint64_t kProcessCommandsId = 82167;
constexpr size_t kMenuStack = 0x110, kNumPauses = 0x160, kFlags = 0x1C;
constexpr uint32_t kPausesGame = 1;
constexpr uint32_t kFreezeFrameGuard = (1u << 5) | (1u << 24);  // kFreezeFrameBackground, kFreezeFramePause

struct MenuDef {
    const char* name;
    uint64_t vtableId;  // AE vtable id (RE::VTABLE_xxx[0])
    uintptr_t vtable;   // resolved
    bool enabled;
};

MenuDef g_menus[] = {
    {"Journal", 216685, 0, false},      // Esc / quests / system
    {"Inventory", 215494, 0, true},
    {"Magic", 215652, 0, true},
    {"Favorites", 215307, 0, true},
    {"Map", 216502, 0, false},
    {"Container", 215061, 0, true},
    {"Crafting", 215111, 0, true},
    {"Barter", 214926, 0, true},
    {"Gift", 215323, 0, true},
    {"Book", 214975, 0, true},
    {"Stats", 215973, 0, false},
    {"Tween", 216145, 0, false},
    {"SleepWait", 215936, 0, true},
    {"Training", 216116, 0, true},
    {"LevelUp", 215560, 0, false},
    {"MessageBox", 215795, 0, false},
    {"Lockpicking", 215604, 0, false},
    {"Tutorial", 216133, 0, false},
    {"Console", 215002, 0, false},
};

bool g_dead = false;
unsigned g_calls = 0;
std::vector<uintptr_t> g_handled;  // allow-listed menu objects whose pause count we already undid
std::vector<uintptr_t> g_seen;     // menu objects already noted in the log

const MenuDef* Match(uintptr_t vt) {
    for (const auto& m : g_menus)
        if (m.enabled && m.vtable == vt) return &m;
    return nullptr;
}

void Reconcile() {
    uint8_t* ui = *reinterpret_cast<uint8_t**>(g_uiPtrAddr);
    if (!ui) return;
    if (!adlite::Readable(ui + kMenuStack, 0x18)) return;
    uint8_t** data = *reinterpret_cast<uint8_t***>(ui + kMenuStack);
    uint32_t size = *reinterpret_cast<uint32_t*>(ui + kMenuStack + 0x10);
    uint32_t cap = *reinterpret_cast<uint32_t*>(ui + kMenuStack + 0x08);
    if (size == 0 || !data) {
        // nothing open: forget the menus we handled; never touch the counter here
        g_handled.clear();
        g_seen.clear();
        return;
    }
    if (size > 32 || cap < size || !adlite::Readable(data, size * 8)) {
        adlite::Log::Write("menuStack looks wrong (size=%u cap=%u data=%p); disabling", size, cap, data);
        g_dead = true;
        return;
    }
    // Forget handled menus that are no longer on the stack (their object may be reused later).
    for (size_t h = 0; h < g_handled.size();) {
        bool present = false;
        for (uint32_t i = 0; i < size; ++i) if (reinterpret_cast<uintptr_t>(data[i]) == g_handled[h]) { present = true; break; }
        if (present) ++h; else g_handled.erase(g_handled.begin() + static_cast<std::ptrdiff_t>(h));
    }
    uint32_t& pauses = *reinterpret_cast<uint32_t*>(ui + kNumPauses);
    for (uint32_t i = 0; i < size; ++i) {
        uint8_t* menu = data[i];
        if (!menu || !adlite::Readable(menu, 0x30)) continue;
        uintptr_t vt = *reinterpret_cast<uintptr_t*>(menu);
        const MenuDef* d = Match(vt);
        if (!d) {  // not on the allow list: never touched, but note it once for the log
            const uintptr_t k = reinterpret_cast<uintptr_t>(menu);
            if (std::find(g_seen.begin(), g_seen.end(), k) == g_seen.end()) {
                if (g_seen.size() > 512) g_seen.clear();
                g_seen.push_back(k);
                for (const auto& m : g_menus) if (m.vtable == vt) adlite::Log::Write("saw menu %s flags=0x%08x (left paused)", m.name, *reinterpret_cast<uint32_t*>(menu + kFlags));
            }
            continue;
        }
        uint32_t& flags = *reinterpret_cast<uint32_t*>(menu + kFlags);
        const uintptr_t key = reinterpret_cast<uintptr_t>(menu);
        if (flags & kFreezeFrameGuard) {  // freeze-frame menus (blurred still of the world) break when the world runs
            if (std::find(g_seen.begin(), g_seen.end(), key) == g_seen.end()) {
                g_seen.push_back(key);
                adlite::Log::Write("saw menu %s with freeze-frame flags 0x%08x (left paused)", d->name, flags);
            }
            continue;
        }
        const bool known = std::find(g_handled.begin(), g_handled.end(), key) != g_handled.end();
        if (!(flags & kPausesGame)) {
            if (!known) g_handled.push_back(key);
            continue;
        }
        flags &= ~kPausesGame;
        if (!known) {
            // The engine counted this menu once when it opened; undo exactly that one count.
            g_handled.push_back(key);
            if (pauses > 0) {
                --pauses;
                adlite::Log::Write("unpaused menu %s flags=0x%08x (numPausesGame now %u, stack=%u)", d->name, flags, pauses, size);
            } else {
                adlite::Log::Write("unpaused menu %s but counter already 0 (stack=%u)", d->name, size);
            }
        } else {
            adlite::Log::Write("menu %s re-set its pause flag; cleared again without touching the counter", d->name);
        }
    }
}

void Detour(void* self) {
    g_orig(self);
    if (g_dead) return;
    ++g_calls;
    if (g_calls <= 3) adlite::Log::Write("hook active (call %u)", g_calls);
    __try {
        Reconcile();
    } __except (EXCEPTION_EXECUTE_HANDLER) {
        g_dead = true;
        adlite::Log::Write("exception in Reconcile; plugin disabled for this session");
    }
}

bool Load(const void*) {
    adlite::Log::Open("AetherialDawn-NoPause");
    adlite::Log::Write("AetherialDawn-NoPause 1.0.5 loading");
    if (!g_lib.Load()) return true;  // never block the game
    g_uiPtrAddr = g_lib.Address(kUiSingletonId);
    uintptr_t target = g_lib.Address(kProcessCommandsId);
    if (!g_uiPtrAddr || !target) { adlite::Log::Write("ids not found; inactive"); return true; }

    // optional ini
    wchar_t exe[MAX_PATH]{};
    GetModuleFileNameW(nullptr, exe, MAX_PATH);
    std::wstring dir(exe);
    dir = dir.substr(0, dir.find_last_of(L"\\/"));
    std::wstring ini = dir + L"\\Data\\SKSE\\Plugins\\AetherialDawn-NoPause.ini";
    for (auto& m : g_menus) {
        m.vtable = g_lib.Address(m.vtableId);
        std::wstring key(m.name, m.name + strlen(m.name));
        m.enabled = GetPrivateProfileIntW(L"Menus", key.c_str(), m.enabled ? 1 : 0, ini.c_str()) != 0 && m.vtable != 0;
        adlite::Log::Write("menu %-12s vtable=0x%llx %s", m.name, (unsigned long long)(m.vtable - adlite::Base()), m.enabled ? "unpause" : "leave");
    }

    uint8_t bytes[16]{};
    if (adlite::Readable(reinterpret_cast<void*>(target), 16)) memcpy(bytes, reinterpret_cast<void*>(target), 16);
    adlite::Log::Write("ProcessCommands at base+0x%llx bytes: %02x %02x %02x %02x %02x %02x %02x %02x %02x %02x %02x %02x",
                       (unsigned long long)(target - adlite::Base()), bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7], bytes[8], bytes[9], bytes[10], bytes[11]);
    if (!adlite::InitHooks() || !adlite::Hook(target, reinterpret_cast<void*>(&Detour), reinterpret_cast<void**>(&g_orig))) {
        adlite::Log::Write("hook failed; inactive");
        return true;
    }
    adlite::Log::Write("hooked UIMessageQueue::ProcessCommands");
    return true;
}

}  // namespace

ADLITE_PLUGIN("AetherialDawn-NoPause", "Aetherial Dawn", 0x01000005, Load)
