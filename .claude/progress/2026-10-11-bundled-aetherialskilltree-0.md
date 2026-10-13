# 2026-10-11 Bundled AetherialSkillTree 0

Bundled AetherialSkillTree 0.1.2 (sha256 E1F0CD14AA0033FC4D6D6A45AEA0440BA35AAA9778641B8C318C16ABD0812225) replacing 0.1.1: 0.1.1 went idle at start because BSString::Set_CStr (id 11044) is not in versionlib-1-6-1170-0.bin; 0.1.2 uses the game allocator and makes each hook optional. Source in native-mods/skill-tree. Checked: offline address-id check and parse/data tests pass, artifact sha verified; NOT runtime verified in game.
