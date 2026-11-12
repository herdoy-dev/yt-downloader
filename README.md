# yt-downloader (TubeGrab)

A small macOS desktop app for downloading videos from YouTube, Facebook,
Instagram, TikTok and X (Twitter). Paste a link into the single box — the site is detected
automatically — pick a resolution and file type, and press Download.

Built with Rust and [Tauri](https://tauri.app); downloads are done by a bundled
copy of [yt-dlp](https://github.com/yt-dlp/yt-dlp).

## Signed-in posts

Instagram (and private or friends-only Facebook posts) only show videos to
signed-in users. Sign in to the site in your browser, then pick that browser
under **Sign-in from** at the bottom of the window; the choice is remembered.
macOS may ask once to let yt-dlp read the browser's saved login.
Safari needs Full Disk Access for this; Brave, Chrome, Firefox and Edge do not.

## Download options

- **MP4 · H.264** – plays everywhere, including QuickTime (up to 1080p)
- **MP4 · H.265** – smaller files, common on TikTok
- **MP4 · AV1** and **WebM · VP9** – higher resolutions and smaller files
- **Audio only** – MP3, M4A or Opus

## Requirements

yt-dlp is included in the app. Two helper tools are used from the system when
present (the app looks in the Homebrew folders):

- `ffmpeg` – needed to combine separate video and sound (all HD options) and for MP3
- `deno` – lets yt-dlp see every format YouTube offers

```sh
brew install ffmpeg deno
```

## Build

Needs Rust and Node.js.

```sh
./build.sh
```

This downloads yt-dlp if needed, builds `TubeGrab.app` and packs it into
`TubeGrab.dmg`.

The app is not notarized. On another Mac, right-click the app and choose Open
the first time.

## Project layout

- `ui/index.html` – the whole interface
- `src-tauri/src/main.rs` – runs yt-dlp and reports progress
- `build.sh` – builds the app and the DMG
