// Offline check of handler.h (no game): installs the handler into a test exe, then
//   handled <dir>  - an access violation caught by the program's own __try  -> a FIRST-CHANCE report, exit 0
//   crash   <dir>  - an access violation nobody catches                    -> FIRST-CHANCE + UNHANDLED reports, process dies
//   stack   <dir>  - runaway recursion                                      -> stack overflow report
//   quiet   <dir>  - a C++ throw/catch and a guarded read (g_skip)          -> NO report, exit 0
// Build: see build-test.bat. Reports go to <dir>\AetherialDawn-Crash.log, never to Documents.
#define ADLITE_STDERR
#include "../handler.h"
#include <stdexcept>

static volatile LONG g_skip = 0;
static void Menus(char* out, size_t n) { strcpy_s(out, n, "Journal, Inventory  [test]"); }
__declspec(noinline) static void Poke(volatile int* p) { *p = 1; }
__declspec(noinline) static int Deep(int d) { volatile char pad[512]; pad[0] = (char)d; return Deep(d + 1) + pad[0]; }

__declspec(noinline) static void CaughtPoke() { __try { Poke(nullptr); } __except (EXCEPTION_EXECUTE_HANDLER) { puts("caught"); } }
__declspec(noinline) static void ThrowCatch() { try { throw std::runtime_error("x"); } catch (...) {} }
__declspec(noinline) static void Quiet() {
    ThrowCatch();
    g_skip = 1;
    __try { Poke(nullptr); } __except (EXCEPTION_EXECUTE_HANDLER) {}
    g_skip = 0;
}

int wmain(int argc, wchar_t** argv) {
    if (argc < 3) return 2;
    if (!crashlog::Install("test", Menus, &g_skip, argv[2])) return 3;
    const std::wstring mode = argv[1];
    if (mode == L"handled") { CaughtPoke(); return 0; }
    if (mode == L"quiet") { Quiet(); return 0; }
    if (mode == L"stack") { Deep(0); return 0; }
    Poke(nullptr);
    return 0;
}
