# Tangleaf

Tangleaf is a local-first personal knowledge app for desktop and Android. Typed pages contain addressable outline or document blocks connected by wikilinks, block references, and backlinks. React and Tauri provide the shared client; Rust, SQLite, and FTS5 keep writing and lexical search available offline; an optional self-hosted Axum server provides realtime sync and all AI functionality.

## What works

- Hierarchical block editing with autosave, Markdown rendering, wikilink and block-reference autocomplete, device-local remembered folding, paragraph splitting, keyboard navigation, and action-based undo/redo.
- Outline and continuous Document authoring, with CodeMirror Source/Live Preview modes, a linked Reading projection, and revision-guarded atomic saves.
- Daily journals with Today/calendar navigation and quick capture, plus loss-aware Logseq graph import for pages, journals, nested blocks, tasks, UUIDs, and local assets.
- A responsive multi-pane workspace with adjacent navigation, linked previews, preserved page sessions, and a collapsible AI companion.
- Local pages, graph, backlinks, attachments, import/export, timestamped recovery backups, and English/Russian FTS5 search. These features work without a server.
- Near-realtime op-based sync over HTTP and WebSocket, with an offline outbox, HLC/LWW conflict resolution, tombstones, deterministic structural reconciliation, snapshot bootstrap, and content-addressed blobs.
- Server-owned semantic retrieval, reranking, entity extraction, and streaming chat. Extracted entities remain disposable server-side AI data rather than client graph nodes; the clients contain no provider keys, vector database, embedding model, or AI background workers.
- Remote AI administration in Settings: provider URLs and models, write-only API keys, capability probes, automatic-indexing and extraction switches, queue progress, failures, and reindex controls.
- A full-workspace graph view, six color palettes with light/dark/system brightness, responsive mobile navigation, Android edge-to-edge safe areas, and configurable native or client window decorations on desktop.
- Handwritten notes as ordinary pages: pen input with pressure and tilt, erasing, lasso editing, persistent undo/redo, binary ink storage in the shared database, versioned sync with explicit conflict comparison, and a native fast-ink path on BOOX e-ink tablets.
- An MCP endpoint on the server that exposes the workspace to outside assistants: create notes, append to journals and pages, rewrite blocks under a revision guard, rename pages, and read by search, subtree, ancestors, backlinks, or neighbors. Writes reach the oplog before the tool answers, so a note added from a phone is on every device moments later.
- Debug-only, localhost-bound Tauri MCP integration for live screenshots, accessibility snapshots, input, logs, and IPC inspection.

Select **New note** or press `Ctrl/Cmd+N` to start writing. Enter a title, then press `Enter` to focus the first block. `Ctrl/Cmd+K` opens search. The graph has its own full-workspace surface; the side companion is reserved for AI.

### Outliner shortcuts

| Shortcut                          | Action                                                       |
| --------------------------------- | ------------------------------------------------------------ |
| `Enter`                           | Save and create the next sibling block                       |
| `Shift+Enter`                     | Insert a newline inside the current block                    |
| `Tab` / `Shift+Tab`               | Indent / outdent                                             |
| `Ctrl/Cmd+Up` / `Ctrl/Cmd+Down`   | Reorder among siblings                                       |
| `Ctrl/Cmd+Enter`                  | Toggle the persisted fold state                              |
| `Backspace` on an empty leaf      | Delete the block                                             |
| `Ctrl/Cmd+Z` / `Ctrl/Cmd+Shift+Z` | Text undo/redo while editing; structural undo/redo elsewhere |

## Architecture

The durable source of truth is a UUID-addressed operation stream. Each client and the server materialize that stream into their own `notes.db`; integer SQLite IDs never cross replica boundaries. Local mutations and remote operations pass through the same idempotent apply engine. Per-field HLC clocks preserve independent title, content, and structure edits, while the sync client batches remote apply and cursor advancement atomically.

Persisted Rust state is exposed through generated tauri-specta bindings and cached in TanStack Query. One typed domain-event adapter invalidates the narrow affected query keys. The Zustand workspace store owns pane layout, navigation, focus, and Assistant-dock projection; `PageSessionRegistry` owns unsaved drafts, conflicts, and linked-reading subscribers. A shared notify layer turns typed command failures into consistent user-visible messages without mixing transient UI state into the backend cache.

The server keeps source pages, blocks, attachments, and the oplog in SQLite. Derived embeddings, generations, indexing jobs, extracted entities, and extraction state live in a separate disposable `ai.db` behind a `VectorStore` interface; they are not synced into the client graph. sqlite-vec is loaded only by the server binary. Provider settings are bootstrapped from the server TOML on first start and subsequently managed from the authenticated application Settings page; provider secrets are stored server-side with owner-only permissions and are never returned to the webview.

See the [documentation index](docs/README.md) for architecture records, the roadmap,
and historical technical investigations.

## Development

Rust builds use the Rust 1.99.1 toolchain pinned in `rust-toolchain.toml`.

Install the [Vite+ CLI](https://viteplus.dev/guide/), then let it provision the pinned Node.js and Bun versions:

```sh
vp install
vp dev
```

Run the desktop client:

On NixOS, first run `direnv allow` in this checkout (or enter `nix develop`).
See [the NixOS guide](docs/nixos.md) for local builds and desktop installation.

```sh
vp run desktop:dev
```

Build the desktop release executable without installer packages:

```sh
vp run tauri build
```

Request Linux packages explicitly when needed:

```sh
vp run tauri build --bundles deb
vp run tauri build --bundles rpm
vp run tauri build --bundles appimage
vp run tauri build --bundles deb,rpm,appimage
```

Initialize and build Android from the same client source:

```sh
vp run tauri android init
just build-android-debug
```

The debug recipe applies `src-tauri/tauri.dev.conf.json`, which enables the global Tauri API and
relaxes `script-src` so the development-only MCP bridge can execute scripts in the WebView. Release
builds keep the strict CSP from `tauri.conf.json`.

The Rust Tauri crate family and tao use released crates, with resolved versions pinned in
`Cargo.lock`. Tauri 2.12 includes the Android activity lifecycle and Wayland decoration fixes that
previously required upstream `dev` patches. The JavaScript API and CLI also use released packages. The checked-in
Android project uses AGP 9.4.1, Gradle 9.8.0 and Kotlin 2.4.21; do not regenerate it with an older
CLI template without reviewing the diff. Built-in Kotlin and the new Android DSL are disabled for
compatibility with existing plugins, matching the upstream AGP 9 migration. Configuration cache is
not enabled: our custom Rust build task still accesses its Project during execution.

The Android settings use `gradle/android-fs.gradle.kts` for android-fs 30.2.0, whose upstream
build script still uses the `kotlinOptions` DSL removed in Kotlin 2.3. The override keeps the
plugin's original source directory and uses `compilerOptions`; remove it once upstream migrates.
The build uses the JDK selected by `JAVA_HOME`; Gradle 9.8 supports JDK 25 and 27.
Android modules and the `buildSrc` plugin target Java 17 bytecode.

Desktop windows are created with their saved decoration mode. On Wayland, GTK negotiates the
native frame with the compositor; borderless windows are created without decorations. Since tao
does not support toggling SSD on existing Wayland windows, changes there are saved for the next
app launch. The current window keeps its active frame and matching controls until restart. Other
desktop backends apply decoration changes immediately. The backend is detected from the native
window handle, not session environment variables.

The only build-time environment input is Tauri's `TAURI_DEV_HOST`, which Vite needs for Android/device hot reload. Product configuration does not read process environment variables.

Theming uses `oklch()` colors, which require Chromium 111+. If a device's palette renders unstyled or colorless (buttons/accents transparent while everything else looks fine), its Android System WebView is too old — update it. If the device has no Google account configured, WebView can be sideloaded directly: download the matching architecture's APK from [APKMirror's Android System WebView page](https://www.apkmirror.com/apk/google-inc/android-system-webview/) and `adb install -r` it; no Play Store/account needed.

### Validation

```sh
vp check
vp test
vp build
cargo fmt --all -- --check
cargo clippy --workspace --locked --all-targets --all-features -- -D warnings
cargo test --workspace --locked --all-features
```

CI also regenerates the tauri-specta TypeScript bindings and fails when the committed contract has drifted from Rust.

### Live UI inspection

Debug builds, desktop and Android alike, include [MCP Server Tauri](https://github.com/hypothesi/mcp-server-tauri). The project-scoped `.codex/config.toml` pins its server. With the development app running, the bridge can inspect the accessibility tree, capture screenshots, click, type, resize, and inspect logs and Tauri IPC.

```sh
vp exec tauri-mcp driver-session start --port 9223
vp exec tauri-mcp webview-dom-snapshot --type accessibility
vp exec tauri-mcp webview-screenshot --file screenshot.png
```

Desktop dev must run via `vp run desktop:dev` (`tauri dev --config src-tauri/tauri.dev.conf.json`), which turns on `withGlobalTauri` so the bridge can reach `window.__TAURI__` and relaxes `script-src` so it can execute scripts; the default `tauri.conf.json` keeps both strict for shipped builds. On Android, build the debug APK with `just build-android-debug` (or pass the same `--config` flag to `tauri android dev`) and forward the port first: `adb forward tcp:9223 tcp:9223`.

The MCP bridge and relaxed development CSP are absent from release builds.

## Configuration and deployment

Client configuration lives only in the typed application Settings file: appearance, window frame, server URL, bearer token, and sync enablement. There are no `.env` files or alternate client configuration paths. If no server is configured, editing, graph navigation, attachments, history, and FTS remain available; sync, semantic search, extraction, and chat are visibly unavailable.

The server uses `server/config.example.toml` as a first-start bootstrap. After that, provider configuration and runtime indexing controls are changed remotely from Settings. OpenRouter is the current embeddings/reranking deployment, while completion can use any OpenAI Chat Completions-compatible or Anthropic Messages-compatible endpoint with a custom base URL.

For rolling upgrades, deploy the server before clients so newly introduced request fields are accepted throughout the transition.

Build a static musl server archive or deploy it through the sibling `cloud-forge` Ansible project:

```sh
just package-server
just deploy-server
```

`just package-server` produces the ignored `release-server/` staging directory and `notes-server-release.tar.gz` archive locally; release binaries are not committed.

TLS and public routing belong to the existing reverse proxy; the notes server binds privately and ships as one static binary plus its bootstrap TOML.

In the `cloud-forge` deployment, the backend and persistent state run on the encrypted home server
`mira`. Moscow keeps TLS termination and proxies both public notes domains over WireGuard to
`10.66.66.2:8787`. Run `just deploy-server` from the home LAN; it uses `site-notes.yml` to update the
home backend and the notes proxy. The complete migration and recovery procedure is documented in
`cloud-forge/docs/NOTES.md`. The former Moscow backend stays disabled.

### Connecting an assistant over MCP

The notes MCP endpoint runs in `notes-server` at `/mcp`, using Streamable HTTP. It accesses the
authenticated user's server workspace; changes sync to connected devices. The desktop or Android
app does not need to stay open, but local changes must sync before an assistant can read them.
This endpoint works independently of the debug-only Tauri MCP bridge on port `9223`, which inspects
the app UI and is absent from release builds.

#### Server configuration

Two settings matter for a public deployment:

```toml
public_url = "https://notes.example.com"
mcp_allowed_hosts = ["notes.example.com", "notes.example.dev"]
```

`mcp_allowed_hosts` is `Host` validation, which the transport applies to protect against DNS
rebinding; without it only loopback is accepted. `public_url` is the canonical origin and turns on
the OAuth endpoints. A workspace reachable under several names describes each of them in its
discovery documents, so any listed name can be connected to; the canonical one stays the issuer.

The reverse proxy must forward `/mcp`, `/oauth/`, and `/.well-known/` to `notes-server`, preserving
the `Host` header and allowing streaming responses without buffering. The sibling `cloud-forge`
project configures this in its `notes_server` and `nginx` roles.

#### Connect Codex with OAuth

Add the server, replacing the example hostname with your deployment's hostname:

```sh
codex mcp add tangleaf --url https://notes.example.com/mcp
```

Codex detects OAuth and opens the authorization page. Enter the same **Server token** used for
sync in Tangleaf Settings, then approve access. The tested setup completed sign-in from this
command alone; a separate `login` command was unnecessary.

For the `cloud-forge` deployment, the token is stored as `vault_notes_server_token` in encrypted
`group_vars/all/vault.yml`. View it locally from that checkout:

```sh
ansible-vault view group_vars/all/vault.yml
```

Use the token on the authorization page; do not paste it into the assistant conversation. In other
deployments, use the original token provisioned for the user in `[[users]]`. A `token_sha256` value
is a digest for server-side verification, not a credential to enter at sign-in.

If authorization was skipped or needs to be repeated:

```sh
codex mcp login tangleaf --scopes mcp,offline_access --oauth-client-registration dcr
```

OAuth issues client-specific access and rotating refresh tokens. The `mcp` scope grants both
reading and writing within the authenticated user's workspace; there is no separate read-only
scope.

Use `/mcp` in Codex to check that `tangleaf` is connected, then ask the assistant to call
`list_pages` with `limit: 3`. A successful list confirms authenticated tool access without changing
notes. `codex mcp get tangleaf` checks the saved configuration, not whether a tool call succeeds.
If the server is missing from the current session's tools, restart the client and check again.

#### Other clients and bearer tokens

For a client with a connector dialog, enter `https://notes.example.com/mcp`, enable OAuth sign-in,
and leave optional client ID and secret fields empty. The server supports dynamic client
registration; the consent page asks for the same server token.

A client that can send an authorization header can also carry the server token directly:

```sh
claude mcp add --transport http tangleaf https://notes.example.com/mcp \
  --header "Authorization: Bearer $TANGLEAF_TOKEN"
```

For Codex, the bearer-token alternative reads a named environment variable:

```sh
codex mcp add tangleaf --url https://notes.example.com/mcp \
  --bearer-token-env-var TANGLEAF_TOKEN
```

`TANGLEAF_TOKEN` must be present in the environment of the Codex process. Choose either OAuth or
the bearer-token configuration for this server. See the
[Codex MCP documentation](https://learn.chatgpt.com/docs/extend/mcp?surface=cli) for client options.

#### Available tools

| Purpose             | Tools                                                                 |
| ------------------- | --------------------------------------------------------------------- |
| List and read notes | `list_pages`, `get_content`, `read_subtree`, `read_ancestors`         |
| Search exact words  | `search_fulltext`                                                     |
| Search by meaning   | `search` (requires a configured server AI provider and a built index) |
| Explore links       | `find_backlinks`, `neighbors`                                         |
| Create and append   | `create_note`, `append_to_journal`, `append_block`                    |
| Edit existing text  | `set_block_content`, `rename_page`                                    |

For example, ask the assistant to find notes about a topic, read a matching page and its blocks,
or append a thought to today's journal. `create_note` reuses an existing page with the same title
and appends the supplied Markdown. `append_to_journal` defaults to today in the server's timezone.

Editing tools are guarded by revisions. Every page and block carries the revision of its editable
field, and a write hands it back; an edit built on text that changed in the meantime is refused
rather than applied, so an assistant working from what it read a minute ago cannot overwrite what
was typed on another device since. See
[Sync, storage, and AI](docs/architecture/sync-storage-ai.md) for the invariants behind that.

#### Troubleshooting

- `401` without credentials is expected. The response should include a `WWW-Authenticate` header
  pointing to OAuth resource metadata. If sign-in fails, check the server token and repeat login.
- If the transport rejects the hostname, add the exact public hostname to `mcp_allowed_hosts`.
- If OAuth discovery fails, check `public_url` and proxy routing for `/.well-known/` and `/oauth/`.
- If semantic search is unavailable, use `search_fulltext`; semantic search needs the server's AI
  configuration and index.
- If a recent local edit is missing, check sync in the app. MCP reads the server replica.

## Remaining work

The core architecture is implemented, but this is still a pre-release project. The main remaining work is:

- cursor-aware oplog compaction and device registration/pairing; snapshots are created and bootstrap works, but the server deliberately does not delete history until it can prove every registered device is past the compaction floor;
- uninterrupted semantic queries while a provider change builds a new vector generation; generation activation is atomic, but the new provider runtime currently becomes authoritative while its generation is building;
- scoped and revocable per-device credentials instead of the current static token mapping;
- Markdown-vault and Obsidian import, plus journal templates, properties, saved queries, and an extension model;
- drag-and-drop block movement, cross-block selection, transclusion, richer Markdown authoring, graph filters/layouts, and larger-corpus performance work;
- signed production packages and end-to-end UI/accessibility regression coverage on desktop and real Android hardware.

NixOS packages and build instructions: [docs/nixos.md](docs/nixos.md).
