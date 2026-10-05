# Scripting: gamemode (`mp` API) and server-side Papyrus

## Contents
1. Verified `mp` API
2. Client-side code strings and `ctx`
3. Events
4. Patterns
5. Anti-patterns
6. Unverified API surface (read the source before citing)
7. Server-side Papyrus
8. Development loop

Source for §1–3: `docs/docs_serverside_scripting_reference.md` and
`docs/docs_events_system.md` (mirrored on the GitBook site). "Server declares
`mp` global variable." "In Skyrim Multiplayer there are no dedicated
clientside scripts." — client logic is shipped as strings inside properties
and event sources. `[V]`

## 1. Verified `mp` API `[V]`

```ts
mp.makeProperty(propertyName: string, options: {
  isVisibleByOwner: boolean;      // false => updateOwner never invoked
  isVisibleByNeighbors: boolean;  // false => updateNeighbor never invoked
  updateOwner: string;            // client JS, runs every update for the PlayerCharacter
  updateNeighbor: string;         // client JS, runs for each synchronized Actor/ObjectReference
}): void;                         // values are saved to the DB automatically

mp.makeEventSource(eventName: string, functionBody: string): void;
// eventName must start with "_"; the server then calls mp[eventName](...)

mp.get(formId: number, propertyName: string): any;          // undefined if unset
mp.set(formId: number, propertyName: string, value: any): void;
mp.clear(): void;                                           // drops added properties and event sources
mp.sendUiMessage(formId: number, message: Record<string, unknown>): void; // to the in-game browser via WebSocket
```

Built-in property names seen in the docs: `"type"`, `"pos"`.
Example: `mp.set(0xff000000, "pos", [0, 0, 0])`.

Property example from the docs:
```js
mp.makeProperty("playerLevel", {
  isVisibleByOwner: true,
  isVisibleByNeighbors: false,
  updateOwner: "ctx.sp.Game.setPlayerLevel(ctx.value || 1)",
  updateNeighbor: ""
});
```

Event source example from the docs:
```js
mp.makeEventSource("_onLocalDeath", `
  ctx.sp.on("update", () => {
    const pl = ctx.sp.Game.getPlayer();
    const isDead = pl.getActorValuePercentage("health") === 0;
    if (ctx.state.wasDead !== isDead) {
      if (isDead) ctx.sendEvent();
      ctx.state.wasDead = isDead;
    }
  });
`);
mp._onLocalDeath = (pcFormId) => { /* server-side handling */ };
```

## 2. Client-side code strings and `ctx` `[V]`

Inside `updateOwner`, `updateNeighbor` and event-source bodies:

| Field | Meaning |
|---|---|
| `ctx.sp` | the SkyrimPlatform API |
| `ctx.refr` | undefined in `makeProperty` options evaluation; the player in `updateOwner`; the neighbor in `updateNeighbor` |
| `ctx.value` | current property value (always undefined during `makeProperty`) |
| `ctx.state` | persistent per-snippet state; "currently shared between properties", so namespace keys |
| `ctx.get(name)` | read another property; reading built-in properties this way is "undefined behavior" |
| `ctx.sendEvent(...)` | fire the event source's server handler |

Use `Game.getFormEx`, not `getForm`, in SP code (FormIDs above 0x7FFFFFFF).

## 3. Events `[V]`

- `mp.onInit(formId)` — can fire several times per formId, including when an
  object loads from the DB. Make handlers idempotent.
- `mp.onUiEvent(formId, data)` — fired on messages from the in-game browser.
- Custom events via `makeEventSource` as above.
- Additional hooks like `onDeath`, `onRespawn`, `onActivate`, `onHit`,
  `onCustomPacket` are `[U]` by name; see §6.

## 4. Patterns

- **Authoritative state lives in properties.** The client only renders
  `ctx.value`. Validate anything the client sends before `mp.set`.
- **Keep update strings tiny and idempotent.** They run "every update" for
  every relevant actor; a loop over the world in `updateNeighbor` scales
  with players squared.
- **`isVisibleByOwner: false` for secrets** (anti-cheat, staff flags).
- **Namespace `ctx.state`** (`ctx.state.myMod_wasDead`) because it is shared
  across properties.
- **Detect client actions with SP hooks** (`hooks.sendAnimationEvent`,
  `hooks.sendPapyrusEvent`) inside an event source, then confirm server-side.
  Papyrus event arguments are not available ("known issue").
- **Use `mp.onInit` to seed defaults**, guarded by `if (mp.get(id, "x") === undefined)`.
- **Send UI state with `sendUiMessage`** and handle replies in `onUiEvent`;
  treat `data` as untrusted input.
- **Reimplement, don't port.** A mod feature that needs quest stages, AI
  packages or SKSE should be rebuilt as properties + events rather than by
  dropping its `.pex` on the server.

## 5. Anti-patterns

- Trusting client-sent gold, damage, level or position without checks.
- Heavy loops or allocations in `on("update")` snippets.
- Unnamespaced `ctx.state` keys.
- Running unmodified mod Papyrus on the server (`data/scripts`).
- Calling non-native Papyrus from SP code (throws since SP 2.7).
- Relying on vanilla cell resets instead of `reloot`.
- Editing `manifest.json` or `_libkey.js`.
- Using `getForm` for high FormIDs.

## 6. Unverified API surface `[U]`

These names have been seen in forks, issues or DeepWiki but their signatures
were not read from `skymp5-server/cpp/addon/ScampServer.cpp` or the TS
typings. Confirm before citing: `place`, `onReinit`, `callPapyrusFunction`,
`createActor`, `getIdFromDesc`, `getDescFromId`, `getLocalizedString`,
`getServerSettings`, `getActorsByProfileId`, `getUserByActor`,
`getActorByUser`, `setEnabled`, `writeLogs`, `executeUiCommand`,
`sendCustomPacket`, `getUserIp`, `registerPapyrusFunction` (`[S]` — exists
per skymp5-functions-lib). Likewise built-in property names beyond `type`
and `pos` (`angle`, `worldOrDirectoryId`, `baseDesc`, `isDisabled`,
`isOpen`, `appearance`, `equipment`, `inventory`, `isDead`, `isOnline`,
`profileId`, `spawnPoint`, …) and the `"6ebd:Skyrim.esm"` desc format.

When the user has the repo: `grep -n "SetMethod\|method(" skymp5-server/cpp/addon/ScampServer.cpp`
and `ls skymp5-server/ts/*.d.ts skymp5-server/ts/types 2>/dev/null` settle this in minutes.

## 7. Server-side Papyrus

- Custom VM in `papyrus-vm/`. Scripts load lazily (`PexScript::Lazy`);
  `AddObject` creates one `ActivePexInstance` per script attached to a
  reference (attachment via plugin VMAD data); events go through
  `VirtualMachine::SendEvent`. `[S]`
- `.pex` files live in `data/scripts` (`data/scripts/source` for `.psc`).
  `[V]`
- Hot reload: `isPapyrusHotReloadEnabled`. Off in production. `[V]`
- Implemented classes (`server_guest_lib/script_classes/`, registered by
  `PapyrusClassesFactory.cpp`) `[S]`: PapyrusActor, PapyrusBook,
  PapyrusCell, PapyrusDebug, PapyrusEffectShader, PapyrusFaction,
  PapyrusForm, PapyrusFormList, PapyrusGame, PapyrusKeyword,
  PapyrusMessage, PapyrusNetImmerse, PapyrusObjectReference,
  PapyrusQuest, PapyrusSound, PapyrusVisualEffect.
- Confirmed natives `[S]`: Actor `RestoreActorValue`, `DamageActorValue`,
  `IsEquipped`; `DrawWeapon`, `UnequipAll`, `PlayIdle` delegated to the
  client via `SpSnippet`. ObjectReference `AddItem`/`RemoveItem` (resolve
  FLST and LVLI), `IsDisabled`, `IsDeleted`, `IsHarvested`. Game
  `GetPlayer`, `FindClosestReferenceOfTypeFromRef`. NetImmerse
  `SetNodeTextureSet`, `SetNodeScale` (broadcast).
- `SpSnippet` modes: `kReturnResult` (returns `Viet::Promise<VarValue>`)
  and `kNoReturnResult`. `[S]`
- Coverage history: the core "only had ~20 most used functions like
  ObjectReference.AddItem and Utility.Wait"; skymp5-functions-lib adds
  vanilla/SKSE mimics and `M.*` helpers (`M.GetPlayersOnline`,
  `M.GetText`). `[V]`
- Known failure strings (old issue mirror) `[S]`: `Method not found -
  'RegisterForAnimationEvent'`, `[onActivate] Error: Out of stack space`.
  A regression test for a vanilla DLC1 script stack overflow exists in
  `misc/tests/`. `[S]`
- Compiling on Linux: the dist ships a `papyrus/` folder; the exact compiler
  path (Wine + Bethesda compiler vs Caprica) is `[U]`. Compile on Windows
  with the CK compiler if in doubt, copy `.pex` to `data/scripts`.

## 8. Development loop

Local: `offlineMode` on, `isPapyrusHotReloadEnabled` on, webpack dev server
on 1234 for the UI, Chromium DevTools on 9000, one client. Production: all
of those off, Master API auth on, a second client to verify sync before
announcing.
