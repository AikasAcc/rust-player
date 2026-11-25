## Installing on Windows x64

If you use v0.4.0, download the Windows x64 ZIP once, extract both executables into a writable folder and open **Rust-Player.exe**. Keep **Rust-Player-Updater.exe** beside it. The standalone versioned EXE is the download used by the in-app updater.

## Playback and library

- Click the speaker to mute and click again to restore the previous level. The slider and scroll wheel let you choose a new level. Software volume is available in Shared mode and when software gain is enabled in Exclusive mode.
- Click a timestamped lyric to seek to that line.
- Turn on **Settings → Appearance → Show album covers beside track titles** for compact covers in the playlist. Missing artwork keeps its square space.
- Drag the window from the free space around search and toolbar controls. Favorites confirmations fade out after three seconds.

## Updates and storage

**Settings → General** lets you check for stable releases at startup or on demand. Choose **Download update** and then **Install and restart**. The player and the helper verify the Windows executable against GitHub’s SHA-256 digest. The helper restores the previous executable if launching the replacement fails.

Settings writes now flush and sync the temporary file before replacing the previous settings. See [planned updates](https://github.com/AikasAcc/rust-player/issues/6) for upcoming library and performance work.
