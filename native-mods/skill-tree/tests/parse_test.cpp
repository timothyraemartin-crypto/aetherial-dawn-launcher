// Offline checks (MSVC): JSON parse of the test file, data sanity, and the settings-table walk against a fake NiTMap.
// Build/run from native-mods\skill-tree: see README. Pass the tests\t folder as argv[1] (default: .\tests\t).
#define ADLITE_STDERR 1
#include "../src/skilltree.cpp"
#include <iostream>
int main(int argc, char** argv) {
    std::string dir = argc > 1 ? argv[1] : "tests\t";
    g_dir = std::wstring(dir.begin(), dir.end());
    int fails = 0;
    auto chk = [&](bool c, const char* m) { if (!c) { ++fails; std::cout << "FAIL " << m << "\n"; } };
    // 1) no settings at all -> test file feeds the state
    g_scanTried = true;
    chk(LoadState(), "load test file");
    chk(g_state.size() == 3 && g_state["blacksmithing"].rank == 2 && g_state["blacksmithing"].perks.count("smith.steel") && g_state["archery"].perks.empty() && g_state["fishing"].rank == 3, "test file content");
    // 2) data sanity
    std::set<int> avs; std::set<std::string> ids; int total = 0;
    for (int i = 0; i < kSkillCount; ++i) {
        const TSkill& s = kSkills[i];
        chk(s.nperks <= s.stars, "perks<=stars"); chk(s.av >= 6 && s.av <= 23 && avs.insert(s.av).second, "av unique"); chk(ids.insert(s.id).second, "id unique");
        total += s.nperks;
        for (int k = 0; k < s.nperks; ++k) chk(ids.insert(s.perks[k].id).second, "perk id unique");
        for (int k = 0; k < s.nextra; ++k) chk(ids.insert(s.extra[k].id).second, "extra perk id unique");
    }
    chk(kSkillCount == 18 && avs.size() == 18, "18 skills 18 avs");
    // 3) fake settings table in the engine's NiTMap layout: vptr, bucket count at +8, bucket array at +0x10, chains {next,key,value}
    {
        static const char* json = "{\"v\":1,\"skills\":{\"woodcutting\":{\"r\":1,\"p\":[\"wood.firewood\"]},\"mining\":{\"r\":1,\"p\":[\"mine.iron\"]}}}";
        static uintptr_t setting3[3] = {0, (uintptr_t)json, 0}, setting1[3] = {0, 0, 0};
        static char gscObj[0x140]; static uintptr_t buckets[401]; static uintptr_t a[3], b[3], c[3];
        a[0] = (uintptr_t)b; a[1] = (uintptr_t)"sSaveGameOldVersion"; a[2] = (uintptr_t)setting1;   // chained in one bucket, first empty
        b[0] = 0; b[1] = (uintptr_t)"sHolidayOldLifeFestival"; b[2] = (uintptr_t)setting3;
        c[0] = 0; c[1] = (uintptr_t)"sSomethingElse"; c[2] = (uintptr_t)setting1;
        buckets[7] = (uintptr_t)a; buckets[300] = (uintptr_t)c;
        *(uintptr_t*)(gscObj + 0x118) = 0x1000; *(uint32_t*)(gscObj + 0x120) = 401; *(uintptr_t*)(gscObj + 0x128) = (uintptr_t)buckets;
        static uintptr_t gscVar = (uintptr_t)gscObj; g_gsc = (uintptr_t)&gscVar;
        g_scanTried = false; for (auto& s : g_settings) s = 0;
        chk(LoadState(), "load from fake settings table");
        chk(g_settings[0] == (uintptr_t)setting1 && g_settings[1] == 0 && g_settings[2] == (uintptr_t)setting3, "found sSaveGameOldVersion and sHolidayOldLifeFestival only");
        chk(g_state.size() == 2 && g_state.count("mining") && g_state.count("woodcutting") && !g_state.count("blacksmithing"), "settings payload preferred over the test file");
        setting3[1] = (uintptr_t)"{\"v\":1,\"skills\":{}}";
        chk(LoadState() && g_state.empty(), "empty skills object: state loads, nothing learned");
    }
    // 4) malformed JSON never crashes
    const char* bad[] = {"", "{", "{\"skills\":5}", "{\"skills\":{\"a\":{\"p\":[1,2]}}}", "[1,2,3]", "{\"skills\":{\"a\":{\"r\":\"x\"}}}"};
    for (auto bj : bad) { P p{bj, bj + strlen(bj)}; J r = p.val(); (void)r; }
    std::cout << "total perks shown " << total << ", fails " << fails << "\n";
    return fails;
}
