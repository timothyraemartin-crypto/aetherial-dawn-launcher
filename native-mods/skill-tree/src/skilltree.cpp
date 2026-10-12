// AetherialSkillTree 0.1.5 (adlite kit, Skyrim 1.6.1170 only).
// Shows our server skills in Skyrim's own Tab constellation menu (StatsMenu):
//  - while the menu is open, the 18 vanilla constellations carry our skill names, perk names and descriptions;
//  - the menu's own list is NEVER reordered or shortened (0.1.2 cut it to the learned skills, but the 3D constellations keep the
//    original order, so names, stars and levels no longer matched: the "jumping around" on 0.1.136). A skill the player has not
//    learned stays in its place and reads "<name> (not learned)", level 0, every star "Locked";
//  - no data from the server => vanilla menu untouched;
//  - skill levels in the menu come from the server rank (15/30/50/75/100) for every learned skill, so no vanilla skill value has to be changed;
//  - perk stars show "learned" from the server state (Actor::HasPerk answers for our perks only while the menu is open);
//  - a learned perk with no star in its vanilla sky (a constellation has 6 to 15 stars, our trees up to 12 perks) is named in the skill's description.
// Server state comes as JSON in an inert GMST (one of three real string settings, see kSettingNames: the client script uses the first that exists) or, for offline tests, from
// Data\SKSE\Plugins\AetherialSkillTree.test.json. Nothing is written to the game's records outside the menu session:
// names are put back when the menu hides. No inventory/equipment calls, no Papyrus, no per-tick work.
// Kill switches (Data\SKSE\Plugins): AetherialSkillTree.off = idle; AetherialSkillTree.keeplevels = unlearned skills keep the game's own
// level (default: 0). AetherialSkillTree.nohide is no longer needed (the list is never filtered) and is ignored.
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
bool g_keepLevels = false;
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

// The inert string GMSTs the client script may hold the server state in, found once by walking the settings table
// (same layout the freeze plugin verified). sTestFile2 does not exist in every install (Bart's skse64.log: "SetGameSettingString:
// sTestFile2 not found"), so the client probes this same list in order and keeps the first one that reads back; here every one
// that exists is remembered and the first holding JSON wins. Keep the list in step with "gmst" in skills-vanilla-map.json.
// 0.1.5: sTestFile1/2/3 do not exist in this game (log: "0 of sTestFile1/2/3 found" among 3619 entries). These three are real string settings from
// Skyrim.esm that Aetherial Dawn never shows (old-save error, Creation Club downloads dialog, a calendar holiday name).
const char* const kSettingNames[] = {"sSaveGameOldVersion", "sOldDownloadsAvailable", "sHolidayOldLifeFestival"};
const int kSettingCount = 3;
uintptr_t g_settings[kSettingCount] = {0, 0, 0}; bool g_scanTried = false;
// The settings table is a NiTMap (BSTCaseInsensitiveStringMap) at GameSettingCollection+0x118: vptr, bucket count (+0x08), bucket array (+0x10);
// each bucket is a chain of {next, key char*, value Setting*}; a Setting is vptr, data union (+0x08, char* for strings), name (+0x10).
// (0.1.2/0.1.4 first drafts scanned for 0x18-byte entries, which is not this layout, so they never found any name; the freeze plugin's
// log shows the real header: vptr, 0x191 buckets, the bucket array.) The offline test builds a fake table in this layout.
void FindSetting() {
    g_scanTried = true;
    uintptr_t gsc = rd<uintptr_t>(g_gsc);
    if (!gsc) { g_scanTried = false; return; }
    uintptr_t base = gsc + 0x118;
    uint32_t cap = rd<uint32_t>(base + 8); uintptr_t buckets = rd<uintptr_t>(base + 0x10);
    if (cap < 16 || cap > (1u << 20) || !buckets || !adlite::Readable(reinterpret_cast<void*>(buckets), static_cast<size_t>(cap) * 8)) {
        adlite::Log::Write("settings table not readable (buckets=%u): only the test file can feed the menu", cap); return;
    }
    uint32_t seen = 0; int found = 0;
    for (uint32_t i = 0; i < cap; ++i) {
        uintptr_t it = rd<uintptr_t>(buckets + static_cast<uintptr_t>(i) * 8);
        for (int hop = 0; it && hop < 64; ++hop, it = rd<uintptr_t>(it)) {
            ++seen;
            uintptr_t key = rd<uintptr_t>(it + 8), val = rd<uintptr_t>(it + 0x10);
            if (!key || !val || !adlite::Readable(reinterpret_cast<void*>(key), 12)) continue;
            const char* k = reinterpret_cast<const char*>(key);
            for (int n = 0; n < kSettingCount; ++n)
                if (!g_settings[n] && _stricmp(k, kSettingNames[n]) == 0) { g_settings[n] = val; ++found; { uintptr_t sp = rd<uintptr_t>(val + 8); char head[13] = {0}; if (sp && adlite::Readable(reinterpret_cast<void*>(sp), 12)) memcpy(head, reinterpret_cast<const char*>(sp), 12); for (int h = 0; h < 12; ++h) if (head[h] && (head[h] < 32 || head[h] > 126)) head[h] = '?'; adlite::Log::Write("found %s setting at %p, value starts '%s'", kSettingNames[n], (void*)val, head); } }
        }
    }
    adlite::Log::Write("settings table: %u buckets, %u entries walked, %d of our setting names found", cap, seen, found);
    if (!found) adlite::Log::Write("none of our setting names exist: only the test file can feed the menu");
}

// Reads the server state; false = no usable data (menu stays vanilla).
bool LoadState() {
    g_state.clear();
    std::string text;
    if (!g_scanTried) FindSetting();
    const char* src = "GMST";
    for (int n = 0; n < kSettingCount && text.empty(); ++n) {
        if (!g_settings[n]) continue;
        uintptr_t s = rd<uintptr_t>(g_settings[n] + 0x08);
        if (s && adlite::Readable(reinterpret_cast<void*>(s), 2) && reinterpret_cast<const char*>(s)[0] == '{') {
            size_t len = 0; const char* c = reinterpret_cast<const char*>(s);
            while (len < 65536 && adlite::Readable(c + len, 1) && c[len]) ++len;
            text.assign(c, len); src = kSettingNames[n];
        }
    }
    if (text.empty()) { text = ReadFileText(g_dir + L"\\Data\\SKSE\\Plugins\\AetherialSkillTree.test.json"); src = "test file"; }
    if (text.empty()) { adlite::Log::Write("no server state (settings empty, no test file): menu stays vanilla"); return false; }
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

// dest = address of a BSFixedString (8 bytes); ours = our text for a learned skill; alt = the text while that skill is not learned.
struct Edit { uintptr_t dest; uintptr_t orig; uintptr_t ours; uintptr_t alt; std::string skill; };
std::vector<Edit> g_edits;
// form -> description while the menu is open. learned = shown for a learned skill, locked = for one the player lacks;
// extra = perks of this skill that have no star (AVIF entries only), named in the text once learned.
struct Desc { std::string skill, learned, locked; std::vector<std::pair<std::string, std::string>> extra; };
std::unordered_map<uintptr_t, Desc> g_desc;
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

bool Learned_(const std::string& skill) { return g_state.count(skill) != 0; }
void AddEdit(uintptr_t dest, const char* text, const char* skill, const char* lockedText) {
    if (!dest || !adlite::Readable(reinterpret_cast<void*>(dest), 8)) return;
    g_edits.push_back({dest, rd<uintptr_t>(dest), MakeFixed(text), MakeFixed(lockedText), skill});
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
        AddEdit(avif + 0x28, s.name, s.id, (std::string(s.name) + " (not learned)").c_str());
        {
            Desc d; d.skill = s.id; d.learned = s.desc; d.locked = std::string("You have not learned this skill. ") + s.desc;
            for (int x = 0; x < s.nextra; ++x) d.extra.emplace_back(s.extra[x].id, s.extra[x].name);
            g_desc[avif] = std::move(d);
        }
        g_avSkill[s.av] = s.id;
        for (size_t k = 0; k < stars.size(); ++k) {
            uintptr_t perk = stars[k].perk;
            g_perkSkill[perk] = s.id;
            if (starPerk[k] >= 0) {
                const TPerk& tp = s.perks[starPerk[k]];
                AddEdit(perk + 0x28, tp.name, s.id, "Locked");
                g_desc[perk] = Desc{s.id, tp.desc, "Learn this skill to unlock its perks.", {}}; g_perkOur[perk] = tp.id;
            } else { AddEdit(perk + 0x28, "Locked", s.id, "Locked"); g_desc[perk] = Desc{s.id, "Reserved for a future perk.", "Learn this skill to unlock its perks.", {}}; g_perkOur[perk] = ""; }
            adlite::Log::Write("  %s star %zu form %08X depth %d grid(%u,%u) -> %s", s.id, k, rd<uint32_t>(perk + 0x14), stars[k].depth, stars[k].gx, stars[k].gy, starPerk[k] >= 0 ? s.perks[starPerk[k]].id : "(locked)");
        }
        adlite::Log::Write("build: %s -> AV %s(%d) '%s' stars %zu perks %d", s.id, Str(rd<uintptr_t>(avif + 0x50)), s.av, s.name, stars.size(), s.nperks);
    }
    adlite::Log::Write("build done: %zu edits, %zu descriptions", g_edits.size(), g_desc.size());
}
void ApplyNames() { for (auto& e : g_edits) *reinterpret_cast<uintptr_t*>(e.dest) = Learned_(e.skill) ? e.ours : e.alt; }
void RestoreNames() { for (auto& e : g_edits) *reinterpret_cast<uintptr_t*>(e.dest) = e.orig; }

// ---------------------------------------------------------------- hooks
typedef void (*DescFn)(void*, void*, void*, uint32_t);
DescFn g_origDesc = nullptr;
void DescDetour(void* self, void* out, void* parent, uint32_t field) {
    if (g_open.load(std::memory_order_relaxed)) {
        auto it = g_desc.find(reinterpret_cast<uintptr_t>(parent));
        if (it != g_desc.end()) {
            const Desc& d = it->second;
            auto st = g_state.find(d.skill);
            std::string text = st == g_state.end() ? d.locked : d.learned;
            if (st != g_state.end()) {  // perks learned in F3 > Skills that have no star in this constellation
                std::string names;
                for (auto& x : d.extra) if (st->second.perks.count(x.first)) { if (!names.empty()) names += ", "; names += x.second; }
                if (!names.empty()) text += " Also learned, with no star in this sky: " + names + ".";
            }
            if (SetBSString(out, text.c_str())) return;
        }
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
    auto st = g_state.find(sk->second);
    if (st == g_state.end()) { if (g_keepLevels) return false; out = 0.f; return true; }  // not learned: level 0, never the game's own
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
uint32_t ProcDetour(void* self, void* msg) {
    uint32_t type = rd<uint32_t>(reinterpret_cast<uintptr_t>(msg) + 8);
    if (type == 1 /*kShow*/ && !g_open.load()) {
        __try {
            if (LoadState()) {
                Build();
                if (!g_edits.empty()) {
                    ApplyNames(); g_open = true;
                    g_logBudget = kVerbose;
                    adlite::Log::Write("menu opened: names applied, learned=%zu", g_state.size());
                    for (auto& kv : g_avSkill) adlite::Log::Write("  AV %d %s: %s", kv.first, kv.second.c_str(), Learned_(kv.second) ? "learned" : "not learned");
                }
            }
        } __except (EXCEPTION_EXECUTE_HANDLER) { RestoreNames(); g_open = false; adlite::Log::Write("exception on show: restored vanilla"); }
    }
    uint32_t r = g_origProc(self, msg);
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
    adlite::Log::Write("AetherialSkillTree 0.1.5 loading");
    wchar_t exe[MAX_PATH]{}; GetModuleFileNameW(nullptr, exe, MAX_PATH);
    g_dir = exe; g_dir = g_dir.substr(0, g_dir.find_last_of(L"\\/"));
    auto has = [](const std::wstring& f) { return GetFileAttributesW((g_dir + L"\\Data\\SKSE\\Plugins\\" + f).c_str()) != INVALID_FILE_ATTRIBUTES; };
    if (has(L"AetherialSkillTree.off")) { adlite::Log::Write("kill switch .off present: idle"); return true; }
    g_keepLevels = has(L"AetherialSkillTree.keeplevels");
    if (!g_lib.Load()) return true;
    CreateThread(nullptr, 0, Init, nullptr, 0, nullptr);
    return true;
}
}  // namespace

ADLITE_PLUGIN("AetherialSkillTree", "Aetherial Dawn", 0x00010005, Load)
