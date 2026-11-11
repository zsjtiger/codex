# web

React + TypeScript frontend for the cloud Codex app. Talks to the Rust gateway
in `../gateway` over HTTP and WebSocket.

## Status

The app covers the workspace features of the gateway: sign-in with a username and
password, conversations with live replies (Markdown, math, charts, code highlighting),
the "Used" panel, file attachments (Markdown, Word, PDF) with a side preview, the
composer's `/` skill menu, Projects, Artifacts, Scheduled tasks, Customize (skills,
connectors, plugins), search, and a personal usage dialog. Real model runs need a
codex runtime configured on the gateway; see `../gateway/README.md`.

## Run

```sh
cd gateway && cargo run          # terminal 1: gateway on 127.0.0.1:8080
cd web && npm install            # terminal 2
npm run dev                      # http://localhost:5173, proxies /api, /healthz, /ws
```

Set `GATEWAY_URL` to point the dev proxy elsewhere.

## Layout

```
src/
├── api/          REST client (`client.ts`), typed resources and shared types
├── realtime/     WebSocket connection to the gateway (`socket.ts`)
├── state/        `useCodex.ts` holds app state and handles live events;
│                 `toolUses.ts` maps app-server items to the "Used" panel
├── components/   screens and widgets: Home, Conversation, Composer, Sidebar,
│                 SignIn, Preview (side panel), Markdown (safe renderer, KaTeX,
│                 charts, highlighting), and the Artifacts, Projects, Scheduled,
│                 Customize, Search and Usage dialogs
├── data/         mock data used by some views
└── styles.css    all styles, with light and dark theme variables
```


## Desktop app (Tauri)

`src-tauri/` wraps this web build in a native window. The app asks for the gateway
address on first launch and keeps it in its local storage. To use a gateway from a build
instead, set `VITE_GATEWAY_URL`. The gateway must list the desktop origin in
`GATEWAY_ALLOWED_ORIGINS`, for example `tauri://localhost,http://tauri.localhost`.

```sh
npm run tauri -- dev            # desktop window on the Vite dev server
npm run tauri -- build --debug  # debug build without installers
```

Linux builds need the WebKitGTK packages (`libwebkit2gtk-4.1-dev`, `libgtk-3-dev`,
`librsvg2-dev`, `libayatana-appindicator3-dev`).
