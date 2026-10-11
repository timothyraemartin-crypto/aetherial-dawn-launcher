#define ADLITE_STDERR 1
#include "../src/skilltree.cpp"
#include <iostream>
int main() {
    g_dir = L"A:\\Aetherial Dawn\\skill-tree-work\\tests\\t"; g_scanTried = true;
    int fails = 0;
    auto chk = [&](bool c, const char* m) { if (!c) { ++fails; std::cout << "FAIL " << m << "\n"; } };
    std::wstring path = g_dir + L"\\Data\\SKSE\\Plugins\\AetherialSkillTree.test.json";
    std::string x = ReadFileText(path);
    std::cout << "read bytes " << x.size() << "\n";
    chk(LoadState(), "load test file");
    chk(g_state.size() == 3, "3 skills");
    chk(g_state["blacksmithing"].rank == 2 && g_state["blacksmithing"].perks.count("smith.steel") && g_state["blacksmithing"].perks.size() == 3, "blacksmithing perks");
    chk(g_state["archery"].perks.empty() && g_state["fishing"].rank == 3, "archery/fishing");
    std::set<int> avs; std::set<std::string> ids; int total = 0;
    for (int i = 0; i < kSkillCount; ++i) {
        const TSkill& s = kSkills[i];
        chk(s.nperks <= s.stars, "perks<=stars"); chk(s.av >= 6 && s.av <= 23 && avs.insert(s.av).second, "av unique"); chk(ids.insert(s.id).second, "id unique");
        total += s.nperks;
        for (int k = 0; k < s.nperks; ++k) chk(ids.insert(s.perks[k].id).second, "perk id unique");
    }
    chk(kSkillCount == 18 && avs.size() == 18, "18 skills 18 avs");
    std::cout << "total perks shown " << total << " fails " << fails << "\n";
    const char* bad[] = {"", "{", "{\"skills\":5}", "{\"skills\":{\"a\":{\"p\":[1,2]}}}", "[1,2,3]", "{\"skills\":{\"a\":{\"r\":\"x\"}}}"};
    for (auto b : bad) { P p{b, b + strlen(b)}; J r = p.val(); (void)r; }
    std::cout << "bad inputs parsed without crash\n";
    return fails;
}
