// AetherialSkillTree 0.1.2 (adlite kit, Skyrim 1.6.1170 only).
// Shows our server skills in Skyrim's own Tab constellation menu (StatsMenu):
//  - while the menu is open, the 18 vanilla constellations carry our skill names, perk names and descriptions;
//  - only skills the player has learned are listed (hidden otherwise); no data from the server => vanilla menu untouched;
//  - skill levels in the menu come from the server rank (15/30/50/75/100) for every learned skill, so no vanilla skill value has to be changed;
//  - perk stars show "learned" from the server state (Actor::HasPerk answers for our perks only while the menu is open).
// Server state comes as JSON in the inert GMST sTestFile2 (set by the client script) or, for offline tests, from
// Data\SKSE\Plugins\AetherialSkillTree.test.json. Nothing is written to the game's records outside the menu session:
// names are put back when the menu hides. No inventory/equipment calls, no Papyrus, no per-tick work.
// Kill switches (Data\SKSE\Plugins): AetherialSkillTree.off = idle; AetherialSkillTree.nohide = names only, all 18 listed.
// AE ids (CommonLibSSE-NG): UI 400327, ActorValueList 400267, StatsMenu vtable 215973, BSFixedString ctor 69161,
// BSString::Set_CStr 11044, TESDescription::GetDescription 14552, Actor::HasPerk 37698, PlayerCharacter singleton 403521,
// GameSettingCollection 400782.
#include "adlite.h"
#include "tree_data.h"
#include <atomic>
#include <cstring>
#include <map>
#include <set>
#include <string>
#include <unordered_map>
#include <vector>

namespace {
adlite::AddrLib g_lib;
std::atomic<bool> g_open{false};
bool g_nohide = false;
uintptr_t g_avl = 0, g_statsVt = 0, g_pcVar = 0, g_gsc = 0;
const int kVerbose = 40;
std::atomic<int> g_logBudget{kVerbose};

template <class T> T rd(uintptr_t a) { T v{}; if (adlite::Readable(reinterpret_cast<void*>(a), sizeof(T))) v = *reinterpret_cast<T*>(a); return v; }
const char* Str(uintptr_t p) { return (p && adlite::Readable(reinterpret_cast<void*>(p), 2)) ? reinterpret_cast<const char*>(p) : "?"; }

// ---------------------------------------------------------------- tiny JSON (objects, arrays, strings, numbers)
struct J {
    enum T { Nul, Num, Str, Arr, Obj } t = Nul;
    double n = 0; std::string s; std::vector<J> a; std::vector<std::pair<std::string, J>> o;
    const J* get(const char* k) const { for (auto& kv : o) if (kv.first == k) return &kv.second; return nullptr; }
};
struct P {
    const char* p; const char* e; bool bad = false;
    void ws() { while (p < e && (*p == ' ' || *p == '\n' || *p == '\r' || *p == '\t')) ++p; }
    bool lit(char c) { ws(); if (p < e && *p == c) { ++p; return true; } return false; }
    std::string str() {
        std::string r; ws(); if (p >= e || *p != '"') { bad = true; return r; } ++p;
        while (p < e && *p != '"') { if (*p == '\\' && p + 1 < e) { ++p; char c = *p; r += c == 'n' ? '\n' : c == 't' ? '\t' : c; } else r += *p; ++p; }
        if (p >= e) bad = true; else ++p;
        return r;
    }
    J val(int depth = 0) {
        J v; ws(); if (p >= e || depth > 8) { bad = true; return v; }
        if (*p == '{') { ++p; v.t = J::Obj; if (lit('}')) return v; for (;;) { std::string k = str(); if (bad || !lit(':')) { bad = true; return v; } J c = val(depth + 1); v.o.emplace_back(k, std::move(c)); if (bad) return v; if (lit(',')) continue; if (lit('}')) return v; bad = true; return v; } }
        if (*p == '[') { ++p; v.t = J::Arr; if (lit(']')) return v; for (;;) { J c = val(depth + 1); v.a.push_back(std::move(c)); if (bad) return v; if (lit(',')) continue; if (lit(']')) return v; bad = true; return v; } }
        if (*p == '"') { v.t = J::Str; v.s = str(); return v; }
        if ((*p >= '0' && *p <= '9') || *p == '-') { char* end = nullptr; v.n = strtod(p, &end); if (end == p) { bad = true; return v; } p = end; v.t = J::Num; return v; }
        if (e - p >= 4 && !strncmp(p, "true", 4)) { p += 4; v.t = J::Num; v.n = 1; return v; }
        if (e - p >= 5 && !strncmp(p, "false", 5)) { p += 5; v.t = J::Num; v.n = 0; return v; }
        if (e - p >= 4 && !strncmp(p, "null", 4)) { p += 4; return v; }
        bad = true; return v;
    }
};

// ---------------------------------------------------------------- server state
struct Learned { std::set<std::string> perks; int rank = 0; };
std::map<std::string, Learned> g_state;  // skill id -> state; filled on each menu open
bool g_haveState = false;
std::wstring g_dir;

std::string ReadFileText(const std::wstring& path) {
    std::string r; FILE* f = nullptr;
    if (_wfopen_s(&f, path.c_str(), L"rb") != 0 || !f) return r;
    char buf[4096]; size_t n; while ((n = fread(buf, 1, sizeof buf, f)) > 0 && r.size() < (1u << 20)) r.append(buf, n);
    fclose(f); return r;
}

// GMST sTestFile2 string, found once by walking the settings table (same layout the freeze plugin verified)
uintptr_t g_setting = 0; bool g_scanTried = false;
void FindSetting() {
    g_scanTried = true;
    uintptr_t gsc = rd<uintptr_t>(g_gsc);
    if (!gsc) { g_scanTried = false; return; }
    uintptr_t base = gsc + 0x118;
    for (int eo = 0; eo < 0x40; eo += 8) {
        uintptr_t entries = rd<uintptr_t>(base + eo);
        if (!entries || entries < 0x10000 || !adlite::Readable(reinterpret_cast<void*>(entries), 0x18)) continue;
        for (int co = 0; co < 0x40; co += 4) {
            uint32_t cap = rd<uint32_t>(base + co);
            if (cap < 256 || cap > (1u << 20) || (cap & (cap - 1))) continue;
            if (!adlite::Readable(reinterpret_cast<void*>(entries), static_cast<size_t>(cap) * 0x18)) continue;
            for (uint32_t i = 0; i < cap; ++i) {
                uintptr_t e = entries + static_cast<uintptr_t>(i) * 0x18;
                uintptr_t key = rd<uintptr_t>(e), val = rd<uintptr_t>(e + 8);
                if (!key || !val || key < 0x10000 || !adlite::Readable(reinterpret_cast<void*>(key), 12)) continue;
                if (_strnicmp(reinterpret_cast<const char*>(key), "sTestFile2", 11) == 0) { g_setting = val; adlite::Log::Write("found sTestFile2 setting at %p", (void*)val); return; }
            }
        }
    }
    adlite::Log::Write("sTestFile2 not found");
}

// Reads the server state; false = no usable data (menu stays vanilla).
bool LoadState() {
    g_state.clear();
    std::string text;
    if (!g_setting && !g_scanTried) FindSetting();
    if (g_setting) {
        uintptr_t s = rd<uintptr_t>(g_setting + 0x08);
        if (s && adlite::Readable(reinterpret_cast<void*>(s), 2) && reinterpret_cast<const char*>(s)[0] == '{') {
            size_t n = 0; const char* c = reinterpret_cast<const char*>(s);
            while (n < 65536 && adlite::Readable(c + n, 1) && c[n]) ++n;
            text.assign(c, n);
        }
    }
    const char* src = "GMST";
    if (text.empty()) { text = ReadFileText(g_dir + L"\\Data\\SKSE\\Plugins\\AetherialSkillTree.test.json"); src = "test file"; }
    if (text.empty()) { adlite::Log::Write("no server state (GMST empty, no test file): menu stays vanilla"); return false; }
    P p{text.data(), text.data() + text.size()};
    J root = p.val();
    const J* sk = root.get("skills");
    if (p.bad || root.t != J::Obj || !sk || sk->t != J::Obj) { adlite::Log::Write("server state unreadable (%s): menu stays vanilla", src); return false; }
    for (auto& kv : sk->o) {
        Learned l;
        if (const J* r = kv.second.get("r")) l.rank = static_cast<int>(r->n);
        if (const J* pp = kv.second.get("p")) for (auto& x : pp->a) if (x.t == J::Str) l.perks.insert(x.s);
        g_state[kv.first] = std::move(l);
    }
    adlite::Log::Write("server state (%s): %zu learned skills", src, g_state.size());
    return true;
}

// ---------------------------------------------------------------- vanilla tree edits
typedef void* (*CtorFn)(void*, const char*);
CtorFn g_ctor = nullptr;
// BSString (data*, u16 size, u16 capacity) written with the game's own allocator (BSString::Set_CStr has no id in 1.6.1170)
typedef void* (*MmGetFn)();
typedef void* (*AllocFn)(void*, size_t, int32_t, bool);
typedef void (*FreeFn)(void*, void*, bool);
MmGetFn g_mmGet = nullptr; AllocFn g_alloc = nullptr; FreeFn g_free = nullptr;
bool SetBSString(void* out, const char* text) {
    if (!g_mmGet || !g_alloc || !g_free || !out || !adlite::Readable(out, 16)) return false;
    size_t len = strlen(text); if (len > 60000) return false;
    char** data = reinterpret_cast<char**>(out); uint16_t* size = reinterpret_cast<uint16_t*>(reinterpret_cast<char*>(out) + 8); uint16_t* cap = reinterpret_cast<uint16_t*>(reinterpret_cast<char*>(out) + 10);
    if (*data && *cap > len) { memcpy(*data, text, len + 1); *size = static_cast<uint16_t>(len); return true; }
    void* mm = g_mmGet(); if (!mm) return false;
    char* fresh = static_cast<char*>(g_alloc(mm, len + 1, 0, false)); if (!fresh) return false;
    memcpy(fresh, text, len + 1);
    char* old = *data; *data = fresh; *size = static_cast<uint16_t>(len); *cap = static_cast<uint16_t>(len + 1);
    if (old) g_free(mm, old, false);
    return true;
}
uintptr_t MakeFixed(const char* s) { uintptr_t slot = 0; g_ctor(&slot, s); return slot; }

struct Edit { uintptr_t dest; uintptr_t orig; uintptr_t ours; };  // dest = address of a BSFixedString (8 bytes)
std::vector<Edit> g_edits;
std::unordered_map<uintptr_t, std::string> g_desc;       // form -> description while the menu is open
std::unordered_map<uintptr_t, std::string> g_perkOur;    // vanilla BGSPerk* -> our perk id (for HasPerk), "" = reserved star
std::unordered_map<uintptr_t, std::string> g_perkSkill;  // vanilla BGSPerk* -> our skill id
std::map<int, std::string> g_avSkill;                    // vanilla AV -> our skill id
bool g_built = false;

struct Star { uintptr_t perk; uintptr_t node; int depth; uint32_t gx, gy; };
void Walk(uintptr_t n, int depth, std::vector<Star>& out, std::set<uintptr_t>& seen) {
    if (!n || depth > 24 || out.size() > 64) return;
    uintptr_t perk = rd<uintptr_t>(n + 0x40);
    if (perk && !seen.count(perk)) { seen.insert(perk); out.push_back({perk, n, depth, rd<uint32_t>(n + 0x4C), rd<uint32_t>(n + 0x50)}); }
    uintptr_t kids = rd<uintptr_t>(n + 0x10); uint32_t kn = rd<uint32_t>(n + 0x20);
    if (kn > 16) return;
    for (uint32_t i = 0; i < kn; ++i) Walk(rd<uintptr_t>(kids + i * 8), depth + 1, out, seen);
}
// Vanilla perks the Lab's existing sync binds to our smith.* perks (skills-vanilla-map.json): keep those stars.
const struct { uint32_t form; const char* id; } kPins[] = {{0xCB40D, "smith.steel"}, {0xCB40E, "smith.dwarven"}, {0xCB40F, "smith.elven"}, {0xCB410, "smith.orcish"},
                                                          {0xCB411, "smith.glass"}, {0xCB412, "smith.ebony"}, {0xCB413, "smith.daedric"}};

void AddEdit(uintptr_t dest, const char* text) {
    if (!dest || !adlite::Readable(reinterpret_cast<void*>(dest), 8)) return;
    g_edits.push_back({dest, rd<uintptr_t>(dest), MakeFixed(text)});
}

void Build() {  // once, at first menu open: decide which star gets which of our perks and prepare the edits
    if (g_built) return; g_built = true;
    uintptr_t avl = rd<uintptr_t>(g_avl);
    for (int i = 0; i < kSkillCount; ++i) {
        const TSkill& s = kSkills[i];
        uintptr_t avif = rd<uintptr_t>(avl + 8 + s.av * 8);
        if (!avif) { adlite::Log::Write("build: AV %d has no AVIF, skill %s skipped", s.av, s.id); continue; }
        std::vector<Star> stars; std::set<uintptr_t> seen;
        Walk(rd<uintptr_t>(avif + 0x118), 0, stars, seen);
        if ((int)stars.size() != s.stars) { adlite::Log::Write("build: %s on AV %d has %zu stars, expected %d: skill left vanilla", s.id, s.av, stars.size(), s.stars); continue; }
        std::stable_sort(stars.begin(), stars.end(), [](const Star& a, const Star& b) { return a.depth != b.depth ? a.depth < b.depth : a.gx != b.gx ? a.gx < b.gx : a.gy < b.gy; });
        std::vector<int> used(s.nperks, 0); std::vector<int> starPerk(stars.size(), -1);
        for (size_t k = 0; k < stars.size(); ++k) {  // pins first
            uint32_t fid = rd<uint32_t>(stars[k].perk + 0x14) & 0x00FFFFFF;
            for (auto& pin : kPins) if (fid == pin.form) for (int q = 0; q < s.nperks; ++q) if (!strcmp(s.perks[q].id, pin.id) && !used[q]) { starPerk[k] = q; used[q] = 1; }
        }
        int q = 0;
        for (size_t k = 0; k < stars.size(); ++k) {
            if (starPerk[k] >= 0) continue;
            while (q < s.nperks && used[q]) ++q;
            if (q < s.nperks) { starPerk[k] = q; used[q] = 1; }
        }
        AddEdit(avif + 0x28, s.name);
        g_desc[avif] = s.desc;
        g_avSkill[s.av] = s.id;
        for (size_t k = 0; k < stars.size(); ++k) {
            uintptr_t perk = stars[k].perk;
            g_perkSkill[perk] = s.id;
            if (starPerk[k] >= 0) {
                const TPerk& tp = s.perks[starPerk[k]];
                AddEdit(perk + 0x28, tp.name); g_desc[perk] = tp.desc; g_perkOur[perk] = tp.id;
            } else { AddEdit(perk + 0x28, "Locked"); g_desc[perk] = "Reserved for a future perk."; g_perkOur[perk] = ""; }
            adlite::Log::Write("  %s star %zu form %08X depth %d grid(%u,%u) -> %s", s.id, k, rd<uint32_t>(perk + 0x14), stars[k].depth, stars[k].gx, stars[k].gy, starPerk[k] >= 0 ? s.perks[starPerk[k]].id : "(locked)");
        }
        adlite::Log::Write("build: %s -> AV %s(%d) '%s' stars %zu perks %d", s.id, Str(rd<uintptr_t>(avif + 0x50)), s.av, s.name, stars.size(), s.nperks);
    }
    adlite::Log::Write("build done: %zu edits, %zu descriptions", g_edits.size(), g_desc.size());
}
void ApplyNames() { for (auto& e : g_edits) *reinterpret_cast<uintptr_t*>(e.dest) = e.ours; }
void RestoreNames() { for (auto& e : g_edits) *reinterpret_cast<uintptr_t*>(e.dest) = e.orig; }

// ---------------------------------------------------------------- hooks
typedef void (*DescFn)(void*, void*, void*, uint32_t);
DescFn g_origDesc = nullptr;
void DescDetour(void* self, void* out, void* parent, uint32_t field) {
    if (g_open.load(std::memory_order_relaxed)) {
        auto it = g_desc.find(reinterpret_cast<uintptr_t>(parent));
        if (it != g_desc.end() && SetBSString(out, it->second.c_str())) return;
    }
    g_origDesc(self, out, parent, field);
}
typedef bool (*HasPerkFn)(void*, void*);
HasPerkFn g_origHasPerk = nullptr;
bool HasPerkDetour(void* actor, void* perk) {
    if (g_open.load(std::memory_order_relaxed) && actor == reinterpret_cast<void*>(rd<uintptr_t>(g_pcVar))) {
        auto it = g_perkOur.find(reinterpret_cast<uintptr_t>(perk));
        if (it != g_perkOur.end()) {
            bool has = false;
            if (!it->second.empty()) { auto sk = g_perkSkill.find(reinterpret_cast<uintptr_t>(perk)); if (sk != g_perkSkill.end()) { auto st = g_state.find(sk->second); has = st != g_state.end() && st->second.perks.count(it->second); } }
            if (g_logBudget.fetch_sub(1) > 0) adlite::Log::Write("HasPerk(%08X) while open -> %d (%s)", rd<uint32_t>(reinterpret_cast<uintptr_t>(perk) + 0x14), (int)has, it->second.c_str());
            return has;
        }
    }
    return g_origHasPerk(actor, perk);
}


// ---------------------------------------------------------------- skill level shown in the menu
// ActorValueOwner vtable slots 1/2/3 = GetActorValue / GetPermanentActorValue / GetBaseActorValue. Only while the menu is open
// and only for the player's 18 skill values; everything else goes to the original.
typedef float (*AvFn)(void*, int);
AvFn g_origAv[3] = {nullptr, nullptr, nullptr};
uintptr_t g_avOwner = 0;  // address of the player's ActorValueOwner subobject
const float kLevels[5] = {15.f, 30.f, 50.f, 75.f, 100.f};
bool OurLevel(void* self, int av, float& out) {
    if (!g_open.load(std::memory_order_relaxed) || reinterpret_cast<uintptr_t>(self) != g_avOwner || av < 6 || av > 23) return false;
    auto sk = g_avSkill.find(av); if (sk == g_avSkill.end()) return false;
    auto st = g_state.find(sk->second); if (st == g_state.end()) return false;
    int r = st->second.rank; if (r < 1) r = 1; if (r > 5) r = 5;
    out = kLevels[r - 1];
    return true;
}
template <int N> float AvDetour(void* self, int av) {
    float v;
    if (OurLevel(self, av, v)) { if (g_logBudget.fetch_sub(1) > 0) adlite::Log::Write("skill value call %d av %d while open -> %.0f", N, av, v); return v; }
    return g_origAv[N](self, av);
}
bool PatchAvOwner() {
    uintptr_t pc = rd<uintptr_t>(g_pcVar);
    if (!pc) { adlite::Log::Write("player not ready, skill level override off"); return false; }
    static const uint64_t ids[] = {208040, 208042, 208044, 208046, 208048, 208050, 208052, 208054, 208056, 208058, 208060, 208062, 208064};
    for (uintptr_t off = 0x90; off <= 0xD8; off += 8) {
        uintptr_t vp = rd<uintptr_t>(pc + off);
        for (uint64_t id : ids) if (vp && vp == g_lib.Address(id)) adlite::Log::Write("player+%02X vptr is PlayerCharacter vtable id %llu", (unsigned)off, (unsigned long long)id);
    }
    // Actor layout (CommonLibSSE-NG, AE): MagicTarget at 0xA0, ActorValueOwner at 0xB8. Accept it only if its vptr is one of PlayerCharacter's own vtables.
    {
        const uintptr_t off = 0xB8;
        uintptr_t vp = rd<uintptr_t>(pc + off);
        bool isVt = false; for (uint64_t id : ids) if (vp && vp == g_lib.Address(id)) isVt = true;
        if (isVt) {
            g_avOwner = pc + off;
            void* thunks[3] = {(void*)&AvDetour<0>, (void*)&AvDetour<1>, (void*)&AvDetour<2>};
            for (int n = 0; n < 3; ++n) {
                auto* slot = reinterpret_cast<void**>(vp + (n + 1) * 8); DWORD old;
                if (!VirtualProtect(slot, 8, PAGE_READWRITE, &old)) return false;
                g_origAv[n] = reinterpret_cast<AvFn>(*slot); *slot = thunks[n]; VirtualProtect(slot, 8, old, &old);
            }
            adlite::Log::Write("skill level override installed (ActorValueOwner at player+%02X)", (unsigned)off);
            return true;
        }
    }
    adlite::Log::Write("ActorValueOwner subobject not identified: skill levels stay as the game has them");
    return false;
}

// StatsMenu: vtable slot 4 = ProcessMessage(UIMessage&)
typedef uint32_t (*ProcFn)(void*, void*);
ProcFn g_origProc = nullptr;
bool FilterTrees(uintptr_t menu) {  // keep only learned skills in the menu's own list
    const uintptr_t rtd = menu + 0x40;
    uintptr_t arr = rd<uintptr_t>(rtd + 0x10); uint32_t n = rd<uint32_t>(rtd + 0x20), cap = rd<uint32_t>(rtd + 0x18);
    if (!arr || n == 0 || n > cap || n > 32 || !adlite::Readable(reinterpret_cast<void*>(arr), n * 4)) return false;
    std::vector<int32_t> keep;
    for (uint32_t i = 0; i < n; ++i) {
        int32_t av = rd<int32_t>(arr + i * 4);
        auto sk = g_avSkill.find(av);
        if (sk != g_avSkill.end() && g_state.count(sk->second)) keep.push_back(av);
    }
    if (keep.empty()) return false;  // nothing of ours learned: do not touch the list
    for (size_t i = 0; i < keep.size(); ++i) *reinterpret_cast<int32_t*>(arr + i * 4) = keep[i];
    *reinterpret_cast<uint32_t*>(rtd + 0x20) = static_cast<uint32_t>(keep.size());
    *reinterpret_cast<uint32_t*>(rtd + 0x2D0) = static_cast<uint32_t>(keep.size());
    return true;
}
uint32_t ProcDetour(void* self, void* msg) {
    uint32_t type = rd<uint32_t>(reinterpret_cast<uintptr_t>(msg) + 8);
    const uintptr_t menu = reinterpret_cast<uintptr_t>(self);
    if (type == 1 /*kShow*/ && !g_open.load()) {
        __try {
            if (LoadState()) {
                Build();
                if (!g_edits.empty()) {
                    ApplyNames(); g_open = true;
                    g_logBudget = kVerbose;
                    adlite::Log::Write("menu opened: names applied, learned=%zu", g_state.size());
                    if (!g_nohide) adlite::Log::Write("menu list filtered=%d", (int)FilterTrees(menu));
                }
            }
        } __except (EXCEPTION_EXECUTE_HANDLER) { RestoreNames(); g_open = false; adlite::Log::Write("exception on show: restored vanilla"); }
    }
    uint32_t r = g_origProc(self, msg);
    if (type == 1 && g_open.load() && !g_nohide) { __try { FilterTrees(menu); } __except (EXCEPTION_EXECUTE_HANDLER) {} }
    if ((type == 3 /*kHide*/ || type == 4 /*kForceHide*/) && g_open.load()) {
        g_open = false; RestoreNames();
        adlite::Log::Write("menu closed: vanilla names restored");
    }
    return r;
}

bool PatchVtable(uint64_t id, int slot, void* thunk, void** orig) {
    uintptr_t vt = g_lib.Address(id);
    if (!vt) return false;
    auto* s = reinterpret_cast<void**>(vt + slot * 8);
    DWORD old; if (!VirtualProtect(s, 8, PAGE_READWRITE, &old)) return false;
    *orig = *s; *s = thunk; VirtualProtect(s, 8, old, &old);
    return true;
}

DWORD WINAPI Init(LPVOID) {
    Sleep(15000);
    g_avl = g_lib.Address(400267); g_statsVt = g_lib.Address(215973); g_pcVar = g_lib.Address(403521); g_gsc = g_lib.Address(400782);
    g_ctor = reinterpret_cast<CtorFn>(g_lib.Address(69161));
    g_mmGet = reinterpret_cast<MmGetFn>(g_lib.Address(11141)); g_alloc = reinterpret_cast<AllocFn>(g_lib.Address(68115)); g_free = reinterpret_cast<FreeFn>(g_lib.Address(68117));
    uintptr_t descFn = g_lib.Address(14552), hasPerkFn = g_lib.Address(37698);
    adlite::Log::Write("ids: avl=%d statsVt=%d player=%d gmst=%d ctor=%d alloc=%d desc=%d hasPerk=%d", g_avl != 0, g_statsVt != 0, g_pcVar != 0, g_gsc != 0, g_ctor != nullptr, g_mmGet && g_alloc && g_free, descFn != 0, hasPerkFn != 0);
    if (!g_avl || !g_statsVt || !g_pcVar || !g_gsc || !g_ctor) { adlite::Log::Write("required address ids missing, plugin idle"); return 0; }
    bool ok = adlite::InitHooks();
    if (descFn && g_mmGet && g_alloc && g_free) adlite::Log::Write("description hook ok=%d", (int)adlite::Hook(descFn, (void*)&DescDetour, (void**)&g_origDesc)); else adlite::Log::Write("description hook skipped (ids missing): vanilla texts stay");
    if (hasPerkFn) adlite::Log::Write("perk-check hook ok=%d", (int)adlite::Hook(hasPerkFn, (void*)&HasPerkDetour, (void**)&g_origHasPerk)); else adlite::Log::Write("perk-check hook skipped (id missing): perk stars show vanilla state");
    ok = ok && PatchVtable(215973, 4, (void*)&ProcDetour, (void**)&g_origProc);
    adlite::Log::Write("hooks installed ok=%d (StatsMenu.ProcessMessage patched; others above)", (int)ok);
    for (int tries = 0; tries < 60 && ok && !PatchAvOwner(); ++tries) Sleep(5000);  // the player object may not exist yet at the title screen
    return 0;
}

bool Load(const void*) {
    adlite::Log::Open("AetherialSkillTree");
    adlite::Log::Write("AetherialSkillTree 0.1.2 loading");
    wchar_t exe[MAX_PATH]{}; GetModuleFileNameW(nullptr, exe, MAX_PATH);
    g_dir = exe; g_dir = g_dir.substr(0, g_dir.find_last_of(L"\\/"));
    auto has = [](const std::wstring& f) { return GetFileAttributesW((g_dir + L"\\Data\\SKSE\\Plugins\\" + f).c_str()) != INVALID_FILE_ATTRIBUTES; };
    if (has(L"AetherialSkillTree.off")) { adlite::Log::Write("kill switch .off present: idle"); return true; }
    g_nohide = has(L"AetherialSkillTree.nohide");
    if (!g_lib.Load()) return true;
    CreateThread(nullptr, 0, Init, nullptr, 0, nullptr);
    return true;
}
}  // namespace

ADLITE_PLUGIN("AetherialSkillTree", "Aetherial Dawn", 0x00010002, Load)
