// AetherialSkillTree PROBE v0 (adlite kit, Skyrim 1.6.1170). READ-ONLY except an optional rename test.
// Watches UI::menuStack; when StatsMenu (the Tab skill constellations) opens it writes what the game keeps for it
// (skill list, selectable count, each skill AVIF's name and perk tree) to Documents\My Games\Skyrim Special Edition\SKSE\AetherialSkillTree.log.
// No inventory/equipment calls, no per-tick game calls beyond reading the menu stack (same poll the freeze plugin uses).
// AE ids (CommonLibSSE-NG): UI 400327, ActorValueList 400267, StatsMenu vtable 215973, BSFixedString::ctor8 69161.
#include "adlite.h"
#include <atomic>
#include <cstring>

namespace {
adlite::AddrLib g_lib;
uintptr_t g_ui = 0, g_avl = 0, g_statsVt = 0;
bool g_rename = false;

template <class T> T rd(uintptr_t a) { T v{}; if (adlite::Readable(reinterpret_cast<void*>(a), sizeof(T))) v = *reinterpret_cast<T*>(a); return v; }
const char* Str(uintptr_t p) { return (p && adlite::Readable(reinterpret_cast<void*>(p), 2)) ? reinterpret_cast<const char*>(p) : "?"; }

uintptr_t FindStatsMenu() {
    uintptr_t ui = rd<uintptr_t>(g_ui);
    if (!ui) return 0;
    uintptr_t data = rd<uintptr_t>(ui + 0x110);
    uint32_t size = rd<uint32_t>(ui + 0x120);
    if (!data || size == 0 || size > 64) return 0;
    for (uint32_t i = 0; i < size; ++i) {
        uintptr_t m = rd<uintptr_t>(data + i * 8);
        if (m && rd<uintptr_t>(m) == g_statsVt) return m;
    }
    return 0;
}

// BGSSkillPerkTreeNode: perk 0x40, gridX 0x4C, gridY 0x50, children BSTArray 0x10 (data,cap,size@0x10)
void DumpNode(uintptr_t n, int depth, int& count) {
    if (!n || depth > 24 || count > 64) return;
    ++count;
    uintptr_t perk = rd<uintptr_t>(n + 0x40);
    adlite::Log::Write("    %*snode idx=%u perk=%p name='%s' grid=(%u,%u) pos=(%.3f,%.3f) kids=%u", depth * 2, "", rd<uint32_t>(n + 8), (void*)perk,
                       perk ? Str(rd<uintptr_t>(perk + 0x28)) : "-", rd<uint32_t>(n + 0x4C), rd<uint32_t>(n + 0x50),
                       rd<float>(n + 0x60), rd<float>(n + 0x64), rd<uint32_t>(n + 0x10 + 0x10));
    uintptr_t kids = rd<uintptr_t>(n + 0x10);
    uint32_t kn = rd<uint32_t>(n + 0x10 + 0x10);
    if (kn > 16) return;
    for (uint32_t i = 0; i < kn; ++i) DumpNode(rd<uintptr_t>(kids + i * 8), depth + 1, count);
}

void DumpMenu(uintptr_t menu) {
    adlite::Log::Write("StatsMenu opened at %p", (void*)menu);
    const uintptr_t rtd = menu + 0x40;  // RUNTIME_DATA (AE)
    uintptr_t arr = rd<uintptr_t>(rtd + 0x10);
    adlite::Log::Write("skillTrees raw: data=%p cap=%u size=%u", (void*)arr, rd<uint32_t>(rtd + 0x18), rd<uint32_t>(rtd + 0x20));
    uint32_t n = rd<uint32_t>(rtd + 0x20);
    for (uint32_t i = 0; i < n && i < 32; ++i) adlite::Log::Write("  skillTrees[%u] = %d", i, rd<int32_t>(arr + i * 4));
    adlite::Log::Write("numSelectableTrees=%u selectedTree=%u flags=0x%x zoomed=%d", rd<uint32_t>(rtd + 0x2D0), rd<uint32_t>(rtd + 0x180), rd<uint32_t>(rtd + 0x2DC),
                       (int)rd<uint8_t>(rtd + 0x178));
    uintptr_t avl = rd<uintptr_t>(g_avl);
    for (int av = 6; av <= 23; ++av) {
        uintptr_t avif = rd<uintptr_t>(avl + 8 + av * 8);
        if (!avif) { adlite::Log::Write("AV %d: no AVIF", av); continue; }
        uintptr_t tree = rd<uintptr_t>(avif + 0x118);
        adlite::Log::Write("AV %d AVIF=%p name='%s' enum='%s' perkTree=%p width=%u", av, (void*)avif, Str(rd<uintptr_t>(avif + 0x28)),
                           Str(rd<uintptr_t>(avif + 0x50)), (void*)tree, rd<uint32_t>(avif + 0x120));
        int count = 0;
        DumpNode(tree, 0, count);
        adlite::Log::Write("  nodes counted: %d", count);
    }
}

// rename test: swap each skill AVIF's FULL name to a fresh BSFixedString. Vanilla order AV 6..23.
const char* kNames[18] = {"One-Handed", "Two-Handed", "Archery", "Block", "Blacksmithing", "Heavy Armour", "Light Armour", "Tailoring", "Leatherworking",
                          "Cooking and Brewing", "Alchemy", "Jewelcrafting", "Mining", "Woodcutting", "Harvesting", "Hunting", "Fishing", "Farming"};
typedef void* (*CtorFn)(void*, const char*);
void RenameAll() {
    auto ctor = reinterpret_cast<CtorFn>(g_lib.Address(69161));
    if (!ctor) { adlite::Log::Write("BSFixedString ctor id missing"); return; }
    uintptr_t avl = rd<uintptr_t>(g_avl);
    for (int i = 0; i < 18; ++i) {
        uintptr_t avif = rd<uintptr_t>(avl + 8 + (6 + i) * 8);
        if (!avif) continue;
        ctor(reinterpret_cast<void*>(avif + 0x28), kNames[i]);  // TESFullName::fullName (old pool ref leaks 1 ref, harmless)
    }
    adlite::Log::Write("renamed 18 skill AVIFs");
}

DWORD WINAPI Poll(LPVOID) {
    uintptr_t last = 0;
    for (;;) {
        Sleep(100);
        uintptr_t m = FindStatsMenu();
        if (m && !last) { DumpMenu(m); }
        last = m;
    }
}

DWORD WINAPI Init(LPVOID) {
    Sleep(15000);
    g_ui = g_lib.Address(400327); g_avl = g_lib.Address(400267); g_statsVt = g_lib.Address(215973);
    if (!g_ui || !g_avl || !g_statsVt) { adlite::Log::Write("address ids missing, idle"); return 0; }
    adlite::Log::Write("ready: ui=%p avl=%p statsVt=%p", (void*)g_ui, (void*)g_avl, (void*)g_statsVt);
    if (g_rename) RenameAll();
    CreateThread(nullptr, 0, Poll, nullptr, 0, nullptr);
    return 0;
}

bool Load(const void*) {
    adlite::Log::Open("AetherialSkillTree");
    adlite::Log::Write("AetherialSkillTree v0.0.2 (names) loading");
    wchar_t exe[MAX_PATH]{}; GetModuleFileNameW(nullptr, exe, MAX_PATH);
    std::wstring dir(exe); dir = dir.substr(0, dir.find_last_of(L"\\/"));
    if (GetFileAttributesW((dir + L"\\Data\\SKSE\\Plugins\\AetherialSkillTree.off").c_str()) != INVALID_FILE_ATTRIBUTES) return true;
    g_rename = GetFileAttributesW((dir + L"\\Data\\SKSE\\Plugins\\AetherialSkillTree.norename").c_str()) == INVALID_FILE_ATTRIBUTES;
    if (!g_lib.Load()) return true;
    CreateThread(nullptr, 0, Init, nullptr, 0, nullptr);
    return true;
}
}  // namespace

ADLITE_PLUGIN("AetherialSkillTree", "Aetherial Dawn", 1, Load)
