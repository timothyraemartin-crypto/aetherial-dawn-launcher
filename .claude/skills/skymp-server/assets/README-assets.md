# Assets

- `server-settings.example.json` - strict JSON. Keys marked verified in
  references/server-settings.md plus `offlineMode` (secondary source; confirm
  in skymp5-server/ts/settings.ts). `ip` is intentionally omitted so the
  server uses its public address.
- `skymp.service` - systemd unit template. Replace WorkingDirectory and
  ExecStart with your dist's real path and start script.
