// Off-Windows check of the menu layout logic (0.1.4): fake AVIF/perk memory, then names, descriptions, levels and restore.
// Run: g++ -std=c++17 -w -I tests/stub -I src tests/layout_test.cpp -o /tmp/layout_test && /tmp/layout_test   (tests/stub/adlite.h stands in for the Windows kit)
#include <iostream>
#include <cstdlib>
#include <sstream>
#include <functional>
#include "../src/skilltree.cpp"
#include <iostream>
#include <cstdlib>
static int fails = 0;
#define CHK(c) do { if (!(c)) { ++fails; std::cout << "FAIL line " << __LINE__ << ": " #c "\n"; } } while (0)
void* fakeCtor(void* slot, const char* s) { *reinterpret_cast<uintptr_t*>(slot) = reinterpret_cast<uintptr_t>(strdup(s)); return slot; }
void* fakeMm() { static char x; return &x; }
void* fakeAlloc(void*, size_t n, int32_t, bool) { return malloc(n); }
void fakeFree(void*, void* p, bool) { free(p); }
void fakeOrigDesc(void*, void* out, void*, uint32_t) { SetBSString(out, "VANILLA"); }
template <class T> void put(void* base, size_t off, T v) { *reinterpret_cast<T*>(reinterpret_cast<char*>(base) + off) = v; }
std::string nameAt(uintptr_t dest) { return reinterpret_cast<const char*>(*reinterpret_cast<uintptr_t*>(dest)); }
std::string descOf(uintptr_t form) { char out[16] = {0}; DescDetour(nullptr, out, reinterpret_cast<void*>(form), 0); return *reinterpret_cast<char**>(out); }
struct FakeSkill { char* avif; std::vector<char*> perks; };
int main() {
    // fake vanilla data: one AVIF per skill AV with exactly s.stars perk nodes
    static uintptr_t avlArr[8 + 64 * 8 / 8 + 8]; memset(avlArr, 0, sizeof avlArr);
    static uintptr_t avlPtr = reinterpret_cast<uintptr_t>(avlArr);
    g_avl = reinterpret_cast<uintptr_t>(&avlPtr);
    std::map<std::string, FakeSkill> fk;
    for (int i = 0; i < kSkillCount; ++i) {
        const TSkill& s = kSkills[i];
        FakeSkill f; f.avif = static_cast<char*>(calloc(1, 0x200));
        put<uintptr_t>(f.avif, 0x28, reinterpret_cast<uintptr_t>(strdup("VanillaName")));
        put<uintptr_t>(f.avif, 0x50, reinterpret_cast<uintptr_t>("EDID"));
        char* root = static_cast<char*>(calloc(1, 0x100));
        char** kids = static_cast<char**>(calloc(16, 8));
        for (int k = 0; k < s.stars; ++k) {
            char* perk = static_cast<char*>(calloc(1, 0x80)); put<uint32_t>(perk, 0x14, 0x100000 + i * 100 + k);
            put<uintptr_t>(perk, 0x28, reinterpret_cast<uintptr_t>(strdup("VanillaPerk")));
            f.perks.push_back(perk);
            char* node = k == 0 ? root : static_cast<char*>(calloc(1, 0x100));
            put<uintptr_t>(node, 0x40, reinterpret_cast<uintptr_t>(perk)); put<uint32_t>(node, 0x4C, k); put<uint32_t>(node, 0x50, k);
            if (k > 0) kids[k - 1] = node;
        }
        put<uintptr_t>(root, 0x10, reinterpret_cast<uintptr_t>(kids)); put<uint32_t>(root, 0x20, s.stars - 1);
        put<uintptr_t>(f.avif, 0x118, reinterpret_cast<uintptr_t>(root));
        avlArr[1 + s.av] = reinterpret_cast<uintptr_t>(f.avif);
        fk[s.id] = f;
    }
    g_ctor = fakeCtor; g_mmGet = fakeMm; g_alloc = fakeAlloc; g_free = fakeFree; g_origDesc = fakeOrigDesc;
    Build();
    CHK(g_edits.size() == 18 + [] { size_t n = 0; for (int i = 0; i < kSkillCount; ++i) n += kSkills[i].stars; return n; }());
    // server state: blacksmithing rank 2 with smith.iron learned + the dropped smith.journeyman; archery rank 1 nothing
    g_state.clear();
    g_state["blacksmithing"].rank = 2; g_state["blacksmithing"].perks = {"smith.iron", "smith.journeyman"};
    g_state["archery"].rank = 1;
    ApplyNames(); g_open = true;
    auto avifName = [&](const char* id) { return nameAt(reinterpret_cast<uintptr_t>(fk[id].avif) + 0x28); };
    CHK(avifName("blacksmithing") == "Blacksmithing");
    CHK(avifName("archery") == "Archery");
    CHK(avifName("mining") == "Mining (not learned)");
    CHK(avifName("farming") == "Farming (not learned)");
    // perk names: learned skill -> our names; unlearned -> Locked
    std::string firstMine = nameAt(reinterpret_cast<uintptr_t>(fk["mining"].perks[0]) + 0x28);
    CHK(firstMine == "Locked");
    std::string firstSmith = nameAt(reinterpret_cast<uintptr_t>(fk["blacksmithing"].perks[0]) + 0x28);
    CHK(firstSmith != "Locked" && firstSmith != "VanillaPerk");
    // HasPerk-visible state: perk -> our id mapping unchanged
    int smithIron = 0; for (auto& kv : g_perkOur) if (kv.second == "smith.iron") ++smithIron;
    CHK(smithIron == 1);
    // descriptions
    std::string dsm = descOf(reinterpret_cast<uintptr_t>(fk["blacksmithing"].avif));
    CHK(dsm.find("Smelting ore") == 0);
    CHK(dsm.find("Also learned, with no star in this sky: ") != std::string::npos);
    CHK(dsm.find("Forgemaster") != std::string::npos);
    CHK(dsm.find("Bellows") == std::string::npos);  // not learned: not named
    std::string dar = descOf(reinterpret_cast<uintptr_t>(fk["archery"].avif));
    CHK(dar.find("Also learned") == std::string::npos);
    std::string dmi = descOf(reinterpret_cast<uintptr_t>(fk["mining"].avif));
    CHK(dmi.find("You have not learned this skill. ") == 0);
    CHK(descOf(reinterpret_cast<uintptr_t>(fk["mining"].perks[1])) == "Learn this skill to unlock its perks.");
    std::cout << dsm << "\n" << dmi.substr(0, 60) << "\n";
    // closed menu -> vanilla
    g_open = false; CHK(descOf(reinterpret_cast<uintptr_t>(fk["mining"].avif)) == "VANILLA"); g_open = true;
    // levels
    static char owner[8]; g_avOwner = reinterpret_cast<uintptr_t>(owner);
    float lv = -1;
    CHK(OurLevel(owner, 10, lv) && lv == 30.f);   // blacksmithing AV 10 rank 2
    CHK(OurLevel(owner, 19, lv) && lv == 15.f);   // archery AV 19 rank 1
    CHK(OurLevel(owner, 9, lv) && lv == 0.f);     // mining, not learned
    CHK(!OurLevel(owner, 30, lv));                 // not a skill AV
    g_keepLevels = true; CHK(!OurLevel(owner, 9, lv)); g_keepLevels = false;
    // restore
    g_open = false; RestoreNames();
    CHK(avifName("mining") == "VanillaName"); CHK(avifName("blacksmithing") == "VanillaName");
    CHK(nameAt(reinterpret_cast<uintptr_t>(fk["blacksmithing"].perks[0]) + 0x28) == "VanillaPerk");
    // none learned -> everything in place, all not learned
    g_state.clear(); ApplyNames(); g_open = true;
    CHK(avifName("blacksmithing") == "Blacksmithing (not learned)");
    OurLevel(owner, 10, lv); CHK(lv == 0.f);
    std::cout << "fails " << fails << "\n";
    return fails;
}
