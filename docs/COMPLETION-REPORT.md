# Rust Player completion report

## Implemented in the local Windows x64 candidate

- Playback, playlists, saved settings, media keys, taskbar controls and listening activity.
- WASAPI Shared and Exclusive, gapless joins for compatible PCM formats, signal-path inspection, optional DSP and software PCM verification.
- Named themes, fonts, local and animated playlist backgrounds, and a background blur setting that is stored with the theme.
- Local lyrics and artwork, Rhai plugins, opt-in Discord artwork lookup and Last.fm scrobbling.

## Verification and remaining checks

| Area | Evidence |
| --- | --- |
| Decoder, gapless and direct PCM | Automated tests compare samples and bytes through the software backend. Hardware verification requires a digital capture from the playback chain. |
| Windows transport | Isolated Windows smoke tests have exercised Previous, Play/Pause, Next and Shuffle through the native thumbnail message path. The manual checklist also covers Explorer and physical media keys. |
| Discord artwork | The public catalog lookup found the cover for the reported track, and the user confirmed that the cover appeared. Other catalog and client combinations can behave differently. |
| Last.fm | Local mock tests exercised authorization, Now Playing and queued submission after restart. Live authentication and submission against a real account remain manual checks. |
| Appearance | Windows test profiles showed that the blur slider softened the background, applying saved the strength, and animated blur changed the background while the artwork panel stayed sharp. Switching songs changed the cover background, and a track without artwork restored the theme color. Automated checks cover persistence, source changes and GIF timing. |
| Performance | The compared players played the same 10,000-track fixture. Visible-window CPU and GPU readings varied between trials, so the isolated cost of animation remains unmeasured. |

Run the [manual release checklist](TESTING.md) on Windows 10 and 11, including device changes and a hardware loopback when a compatible device is available. Video backgrounds, ASIO, DSD/DoP, WavPack and Monkey's Audio remain outside this candidate. Versioning and publication require maintainer approval.
