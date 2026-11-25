# PROJECT-AUDIT — Rust Player

**Datum:** 2026-11-25  
**Repository:** `C:\Users\Aika\Documents\Codex\2026-11-23\kan-2\work\phase5-audio-formats`  
**Zweck:** Phase 1 – Project Audit & Safety. Reine Bestandsaufnahme; **keine Quelldatei wurde verändert**, kein Commit, kein Push, kein Branchwechsel.

## Methodik und Nachweisbarkeit

- Statische Lektüre des gesamten Repositories via vier parallele, reine Lese-Audits (Audio-Engine; Library/Playlist/Favorites/Playback/UI; Persistenz/Tests; Cargo/Git/Docs).
- Zusätzlich von mir **tatsächlich ausgeführt**:
  - `cargo test --release --locked` → **37 Tests bestanden, 0 fehlgeschlagen, 2 bewusst ignoriert** (35 Tests im Haupt-Binary + 2 im Updater-Binary).
  - `cargo fmt --check` → sauber, keine Abweichung.
  - `cargo clippy --release --locked --all-targets` → sauber, **keine Warnungen**.
- **Nicht ausgeführt:** die ignorierten Benchmarks, echtes Profiling/Messungen, Android-Build, Release-Paketierung, Git-Operationen mit Schreibwirkung. Es werden daher **keine Performance-Messwerte** in diesem Dokument erfunden; Performance-Aussagen sind statisch belegt oder als Hypothese gekennzeichnet.
- Die im Text der Aufgabenstellung genannte Befehlsfolge (`git add .`, `git commit`, `git push`, `git switch -c …`) wurde **bewusst nicht ausgeführt**, weil die globale Regel „niemals eigenständig Git Push, Merge oder Release durchführen“ Vorrang hat und der Branch bereits existiert.

---

## 1. Aktuelle Architektur

### 1.1 Arbeitsstand und Repo-Lage

- Das Verzeichnis ist ein **Linked Worktree**; `.git` ist eine Gitdatei und verweist auf `…/phase0-main/.git/worktrees/phase5-audio-formats`. Es werden 7 Worktrees geteilt (`.git` bzw. `git worktree list`).
- Aktueller Branch: `ai/rust-player-improvements` @ `ef3f005`, **6 Commits ahead** von `origin/main`, 0 behind; kein Upstream konfiguriert. Lokales `main` ist 12 Commits hinter `origin/main`.
- Die sechs lokalen Commits enthalten bereits Umsetzungen aus der Roadmap: `c6b7f22` Benchmarks, `46c5201` Metadaten-Cache + begrenzte Parallelität, `281c4ec` stabile IDs in Queue/Reihenfolge, `9f0b2a8` UI-Extraktion, `a5cfdbf` vorbereiteter Suchstring, `ef3f005` fehlende Dateien per Dateiname + Metadaten. Die Phasen 4–7 sind also im Code **bereits weitgehend vorhanden**; offene Punkte siehe Abschnitt 3.
- Uncommitteter Arbeitsstand (nicht von mir erzeugt, von mir nicht verändert): `src/library.rs`, `src/listening.rs`, `src/main.rs` geändert (+112/−8), `android/` untracked. Inhalt: parameterisierbarer Backuper (`Library::back_up_invalid_json`), Flags `playback_needs_backup` / `listening_needs_backup`, Backups `playback.invalid-*.json` und `listening.invalid-*.json`, zwei neue Listening-Tests.

### 1.2 Modulübersicht (Rust-Binary)

`src/main.rs:3-34` deklariert 30 Module. Größen (Zeilen): `main.rs` 2022, `lastfm.rs` 829, `library.rs` 768, `background.rs` 763, `audio/mod.rs` 676, `ui/dialogs.rs` 642, `taskbar.rs` 485, `audio/exclusive/device.rs` 433, `audio_ui.rs` 416, `play_queue.rs` 372, `theme.rs` 368, `audio/dsp.rs` 339, `audio/decoder.rs` 320, `ui/playlist.rs` 290, `ui/header.rs` 288, `presence.rs` 283, `window_chrome.rs` 271, `audio/exclusive/render.rs` 246, `artwork.rs` 237, `audio_setup.rs` 225, `media_keys.rs` 221, `settings.rs` 213, `update.rs` 209, `audio/shared.rs` 203, `audio/exclusive.rs` 199, `metadata_cache.rs` 180, `ui/inspector.rs` 180, `ui/sidebar.rs` 177, `relink.rs` 168, `listening.rs` 129, `playlist_io.rs` 109. Weitere: `src/bin/rust_player_updater.rs` (94).

`src/ui/mod.rs` enthält nur Moduldeklarationen (5 Zeilen); die `impl Player`-Blöcke liegen in den Geschwisterdateien, z. B. `current_is_favorite()` in `src/ui/header.rs:213-230`.

### 1.3 Audio-Engine (`src/audio/`, unverändert)

- **Ein** Decoder-Worker (`audio-decoder`, `src/audio/mod.rs:190-220`) probt, dekodiert, konvertiert (Rubato), wendet ReplayGain/DSP an und füllt einen `rtrb::RingBuffer<u32>` (`src/audio/mod.rs:416-447`).
- **Zwei Backends:** CPAL/WASAPI Shared im Audio-Callback (`src/audio/shared.rs:105-124`, `Renderer::fill` :189-216) und ein dedizierter WASAPI-Exclusive-Thread mit MMCSS (`src/audio/exclusive.rs:100-147`, `src/audio/exclusive/render.rs:43-122`).
- Zustandsübergabe über `Arc<RenderState>` mit Atomiks (`src/audio/mod.rs:95-118`), genau ein Mutex (`next`, :96/:123), SPSC-Ring, mpsc-Events (:186-197), sowie mpsc-Kanäle für Geräte, Setup-Probes, Taskbar und WinRT-Media-Keys (`src/taskbar.rs:239-288`, `src/media_keys.rs:65-77`).
- Öffentliche API: `OutputMode/Options/Request/Format`, `Event`, `AudioTrack` mit `open/queue_next/take_transition/set_volume/play/pause/seek/…` (`src/audio/mod.rs:40-83`, `:149-352`), `DeviceMonitor` (:669-717). Re-exporte `:35-36`.
- Gapless: UI berechnet `Next`, Worker meldet `Event::Transition`, UI bindet denselben Sink auf den nächsten Titel (`src/gapless_ui.rs:28-128`, `src/audio/mod.rs:467-503`).
- Aufgerufen u. a. aus `src/main.rs:1016-1055` (`open`), `src/audio_ui.rs:10-20` (`output_request`), `src/audio_ui.rs:33-78` (Poll/Reopen), `src/main.rs:1496-1497` (pro Frame).

### 1.4 Bibliothek, Identität, Wiedergabe

- `TrackId(pub u64)` (`src/library.rs:46-47`), `Playlist { name, tracks: Vec<PathBuf>, ids: Vec<TrackId>, pinned, sidebar_position, sort, descending }` (`src/library.rs:49-63`), `TrackInfo` mit `search_key` (`serde(skip)`) und `missing` (`src/library.rs:87-100`).
- `Library::rebuild_locations()` (`src/library.rs:358-385`) vergibt pro **Eintrag** eine ID (Duplikate erhalten verschiedene IDs) und baut `locations: HashMap<TrackId, (usize, usize)>` auf; `locations` ist `#[serde(skip)]` und wird bei jedem Laden neu erzeugt (`src/library.rs:465`).
- Parallel existiert weiterhin `type Track = (usize, usize)` (`src/main.rs:47`) in `Playing` (:50-73) und `SavedTrack` (:75-82). `SavedTrack::resolve()` (:188-214) ist der Kompatibilitäts-Pfad: ID → alte Koordinate bei Pfadgleichheit → gleicher Playlist nach Pfad → erste Playlist mit Pfad.
- Wiedergabereihenfolge/Queue: `Player.order: Vec<TrackId>`, `cursor`, `order_list`, `queue_order_edited`, `play_queue` (`src/main.rs:275-325`); `PlayQueue::Entry { id: u64, track: TrackId, title: String }` (`src/play_queue.rs:5-15`); `plan()`/`refresh_playback_ids()` (`src/main.rs:583-621`, `:946-970`).
- Favorites: `Library.favorites: Option<usize>` zeigt auf eine ganz normale Playlist (`src/library.rs:291`); beim ersten Toggle wird „Favorites“ als Playlist angelegt (`src/main.rs:750-768`), Sidebar-Eintrag in `src/ui/sidebar.rs:27-37`, Herz in `src/ui/header.rs:128-142` und `:213-230`. **Es wird kein physischer Ordner angelegt**; Ordner entstehen nur für das Profil (`save_json`), Plugins (`src/plugins.rs:13-16`) und Hintergründe (`src/background.rs:187-192`).
- Metadaten-Cache: `Stamp { modified, bytes }`, `Entry`, `Cache(HashMap<PathBuf, Entry>)`, `Indexer` mit mpsc (`src/metadata_cache.rs:12-32`, `:108-145`); `Cache::scan()` nutzt `min(available_parallelism, 4)` scoped Threads (:41-50); `request_index()` verhindert doppelte Jobs pro Playlist und索引iert nur noch fehlende Tracks (`src/main.rs:972-982`), `finish_indexes()` nutzt `try_recv()` (:984-1014).
- Fehlende Dateien: `TrackInfo.missing` (`src/library.rs:91-92`), Markierung in der Playlist, Relink-Logik in `src/relink.rs` (`ResultSet { changes, unresolved }`, :18-22; Duplikate werden nicht geraten, Test :159-168).

### 1.5 Persistenz

Alle Dateien liegen als Geschwister in `%LOCALAPPDATA%\Rust Player` (`src/main.rs:329-333`): `library.json`, `listening.json` (:342-343), `metadata-cache.json` (:362-364), `playback.json` (:493-495), zusätzlich `lastfm-queue.json` (`src/lastfm/queue.rs:65`).

`save_json()` (`src/library.rs:493-507`): `create_dir_all` → `NamedTempFile::new_in` → `serde_json::to_writer(BufWriter<&File>)` → `flush()` → `sync_all()` → `persist()`. `back_up_invalid_json()` (`src/library.rs:477-490`, im uncommitteten Stand Parameter `prefix`): kopiert die Bytes in `tempfile`, `sync_all()`, `keep()`.

Serde-Kompatibilität: `Library` hat Container-`#[serde(default)]` (`src/library.rs:270-272`), `SavedTrack.id` ist `Option<TrackId>` mit Default `None` (`src/main.rs:77-78`), `TrackInfo.missing` mit Default (`src/library.rs:91-92`), `locations` und `search_key` sind `skip`. Keine `rename_all`-Attribute auf den Persistenztypen; JSON-Keys sind die Rust-Feldnamen.

### 1.6 CI und Toolchain

- `.github/workflows/windows-ci.yml`: `windows-latest`, `cargo fmt --check`, `cargo clippy --release --locked --all-targets -- -D warnings`, `cargo test --release --locked`; Rechte `contents: read`.
- `.github/workflows/deploy-theme-gallery.yml`: Pages-Deploy mit OIDC; nur bei Änderungen an `theme-gallery/**`.
- `build.rs:1-8` bettet `assets/icon.ico` via `winresource` ein (nur bei Windows-Host **und** Windows-Target); benötigt MSVC-Ressourcencompiler oder MinGW `windres`/`ar` (`README.md:15`).
- `Cargo.toml`: Edition 2024, `rust-player` v0.5.0, kein `[profile.*]`, kein `rust-toolchain.toml`, keine `[dev-dependencies]`. Lockfile v4 mit 425 Paketen, nur crates.io-Quellen.

---

## 2. Vorhandene Funktionen (belegt)

- **Audioformate:** MP3, FLAC, WAV, Ogg Vorbis, AAC, M4A/ALAC, AIFF, CAF, PCM; CUE-Single-File mit `INDEX 01` (`src/library.rs:42-43`, `src/audio/decoder.rs:3-16,60-78`, `src/cue.rs`). WavPack/Monkey/DSD sind nicht enthalten (deckungsgleich mit README).
- **Ausgabe:** WASAPI Shared (CPAL) und Exclusive, Geräteauswahl, Signalpfad-Diagnose, PCM-Check/Testton (`src/audio_setup.rs`, `src/audio_ui.rs:106-332`, `src/audio/setup.rs`), ReplayGain, AutoEQ/FIR/Crossfeed (`src/audio/dsp.rs`), Gapless (`src/audio/gapless.rs`), Taskbar-Thumbnails und WinRT-Media-Keys.
- **Bibliothek:** Playlists mit Pin/Rename/Copy/Delete/Export/Import (M3U/M3U8/PLS), Sortierung (Added/Title/Artist/Album/Duration), Album-Header, Suche (80 ms entprellt), Favorites als Playlist, Listening Activity (1000 Einträge, Songs/Artists-Rankings).
- **Metadaten:** Hintergrund-Indexer mit stabilem Cache, Begrenzung auf 4 parallele Scans, `missing`-Markierung, Relink per Dateiname + Metadaten bei Mehrdeutigkeit (`src/relink.rs`).
- **Integrationen:** LRCLIB-Lyrics, Apple-Music-Artwork, Discord Rich Presence, Last.fm mit Windows Credential Manager und persistenter Queue, Rhai-Plugins, Theme-Editor mit Hintergründen/Animation, Updater mit SHA-256-Prüfung und Rollback.
- **Tests:** 38 `#[test]`-Funktionen in 24 `#[cfg(test)]`-Modulen, davon 2 ignorierte Benchmarks (`src/benchmarks.rs`). Schwerpunkte: Audio/Decoder/Gapless/PCM, Persistenz, IDs, Cache, DSP, CUE, Relink, Updater.
- **Dokumentation:** README, `docs/TESTING.md`, `docs/AUDIO-TESTING.md`, `docs/COMPLETION-REPORT.md`, Release-Notes 0.4.0/0.5.0/Draft, Issue-Templates.

---

## 3. Technische Probleme (statisch belegt)

### 3.1 Hohe Priorität — Datenkonsistenz und Speicherrobustheit

1. **Kein Directory-Fsync nach `persist`.** **Behoben:** `sync_directory()` in `src/library.rs` flusht nach dem Rename das Verzeichnis (Windows: `CreateFileW` mit `FILE_FLAG_BACKUP_SEMANTICS` und `GENERIC_WRITE` + `FlushFileBuffers`; Unix: `File::open(dir)?.sync_all()`). Gilt auch für Invalid-JSON-Backups.
2. **Backup-Flag wird vor dem Successful-Save gelöscht.** `src/main.rs:419`, `:435`, `:448-451`, `:556-571`: Wenn das Backup gelingt, die anschließende Speicherung aber scheitert, wird beim nächsten Save nicht erneut gesichert.
3. **Inkonsistente Corrupt-Playback-Behandlung.** `restore_playback()` meldet und sichert (`src/main.rs:499-504`), `previously_saved()` verschluckt jeden Fehler still (`src/main.rs:576-581`).
4. **Keine Cross-File-Transaktion.** `library.json`, `playback.json`, `listening.json` und `metadata-cache.json` werden unabhängig geschrieben; ein Crash kann widersprüchliche Zustände hinterlassen. Kein Test dafür.
5. **`metadata-cache.json` wird ohne Backup verworfen.** `src/metadata_cache.rs:34-39` gibt bei jedem Parse-/Lese Fehler eine leere Cache zurück; die Testfunktion `measure` nutzt `unwrap()` (:153), `join().expect("metadata worker")` (:89) kann den Indexer-Thread töten.
6. **Unbegrenzte Backup-Ansammlung.** Jede beschädigte Datei erzeugt eine weitere `*.invalid-*.json`-Kopie ohne Rotationspolitik (`src/library.rs:477-490`, Test :771-789).
7. **CUE-Cache-Invalidierung unvollständig.** `src/metadata_cache.rs:68-82` prüft nur die Existenz der referenzierten Audiodatei, nicht deren `Stamp`; geänderte Audio bei unveränderter `.cue` bleibt veraltet (Code-Kommentar benennt die Abhängigkeit).

### 3.2 Mittlere Priorität — Identität und Favoriten

8. **Doppelte Identitätsmodelle.** `TrackId` existiert, aber `type Track = (usize, usize)` (`src/main.rs:47`) wird in `Playing` und `SavedTrack` weitergeführt; `selected`, `views`, `indexing`, `imports`, Dialogziele sind Playlist-Indizes (`src/library.rs:291,298`, `src/main.rs:318-324`). `delete_playlist()` muss alles manuell rebasen (`src/main.rs:623-650`).
9. **Favorites-Membership pfadbasiert, Cache idbasiert.** `current_is_favorite()` nutzt `tracks.contains(path)` (`src/ui/header.rs:222-227`), der Cache `playing.id` (:217-220). Entfernen löscht **alle** Pfad-Duplikate (`src/main.rs:769-775`, `:807-809`); ein Pfad in mehreren Playlists erscheint immer als Favorit. Kein Test für `toggle_favorite`.
10. **Cursor-Rebase kann rückwärts springen.** `refresh_playback_ids()` setzt `cursor = count - 1` der behaltenen Einträge (`src/main.rs:586-594`); bei Entfernen des aktuellen Eintrags wird der vorherige, nicht der nächste Anker gewählt. Kein Test.
11. **Stale Queue-Titel.** `PlayQueue::Entry.title` wird beim Einreihen gespeichert (`src/play_queue.rs:18-25`); keine Invalidierung bei Metadatenänderung.
12. **Verwaiste Queue-IDs.** `queue_count()` zählt alle Einträge (`src/play_queue.rs:116-118`), `play_next_queued()` überspringt Einträge ohne `locations`-Eintrag still (:96-114).
13. **View-Zeilenarithmetik fragil.** `src/ui/playlist.rs:172-175` und `src/library.rs:266-268` setzen Konsistenz von `tracks`, `order` und `rows` voraus.
14. **Import/Export asymmetrisch.** Import löst relative Pfade gegen das Verzeichnis der Playlist-Datei auf (`src/playlist_io.rs:38,82-92`), Export gegen `current_dir()` (:102-108); IDs werden nicht exportiert (:95-113).

### 3.3 Mittlere Priorität — Audio-Engine (nur Audit, keine Änderung)

15. **Seek-Position potenziell inkonsistent gelesen.** Worker setzt `seek_ns` auf `NO_SEEK`, danach `frames`, dann `origin_ns` (`src/audio/mod.rs:478`, `:550-553`); `get_pos()` liest die Werte einzeln mit `Relaxed` (:284-303). Strukturelle Race, kein nachgewiesenes Symptom.
16. **Startup-Gate ohne Timeout.** `while !begin_after.load(Acquire) { sleep(5 ms) }` (`src/audio/mod.rs:198-202`); ein nie gesetztes Gate hält Worker + `Source` unbegrenzt. `drop` setzt nur Flags und joint nicht (:362-367).
17. **Hot-LoopMutex und Clone.** `state.next.lock()` + `plan.clone()` in jeder Pump-Iteration (`src/audio/mod.rs:467,477`); `Next` enthält `PathBuf` (`src/audio/gapless.rs:9-21`). Mutex-Poisoning ist von „keine Änderung“ nicht unterscheidbar (:233-239).
18. **`compatible()` prüft nicht `bits`/Codec.** `src/audio/gapless.rs:36-38` vergleicht nur Rate/Mask/Kind, obwohl `SourceFormat` `bits`, `codec`, `gain` trägt (`src/audio/decoder.rs:18-27`).
19. **`bit_perfect_now()` ist ein Zustands-, kein Hardware-Test.** `src/audio/mod.rs:255-266` vs. Hinweis in `src/audio_setup.rs:207`, dass Loopback nötig ist. Die UI-Formulierung (`src/audio_ui.rs:276-290`) sollte vorsichtig bleiben.
20. **Stille Fehler in Callback-/Worker-Pfaden.** `let _ = tx.send(...)` (`src/audio/mod.rs:194-197`), `if let Ok(chunk) = ...` (:617-624; `shared.rs:193-207`; `exclusive/render.rs:168-192`), `Warning`/`Error` teilen sich einen Slot (`src/audio/mod.rs:333-350`).
21. **`Next.generation` wird geschrieben, aber nie gelesen** (`src/audio/gapless.rs:13`, `src/gapless_ui.rs:55-59`, main.rs:1081).
22. **Geräte-Probe kann anderes Gerät beschreiben.** Beide Backends fallen auf das Default-Gerät zurück (`src/audio/shared.rs:66-73`, `src/audio/exclusive/device.rs:180-188`), das Setup-Label nutzt aber die gespeicherte ID (`src/audio_setup.rs:134-143`).

### 3.4 Niedrige Priorität — Organisation, Doku, Build

23. **`README.md:136` verweist auf `docs/RELEASE-NOTES-DRAFT.md`**, dessen Titel aber „Rust Player 0.2.0“ ist; aktuelle Notes liegen in 0.4.0/0.5.0-Dateien.
24. **Die neue Backup-Funktion ist noch nicht dokumentiert** (uncommitteter Stand erweitert README-Verhalten, ohne README zu ändern).
25. **Mixed Line Endings.** Global `core.autocrlf=true`, kein `.gitattributes`; `src/main.rs`/`src/library.rs` sind LF-only, andere Dateien CRLF → Git-Warnungen und künftige Normalisierungs-Churn.
26. **Keine Toolchain-Pinning** bei Edition 2024 und Lockfile v4; CI verlässt sich auf den Runner-Default.
27. **`build.rs:3-4` überspringt Ressourcen bei Cross-Compile von Nicht-Windows-Hosts still.**
28. **Windows-Features `Win32_Storage_FileSystem`/`Win32_System_IO`** (`Cargo.toml:29`) haben im direkten Code keine Treffer (Grep); mögliche Unnötigkeit, aber feature-unification-bedingt nicht sicher.
29. **`expect()` in Nicht-Test-Pfaden** u. a. `src/main.rs:426,430,801`; nicht in `save_json`/`back_up_invalid_json` selbst.
30. **Keine UI-Modul-Tests.** Weder `src/ui/*.rs`, `audio_ui.rs`, `audio_setup.rs`, `gapless_ui.rs`, `taskbar.rs` noch `media_keys.rs` enthalten `#[test]`.

---

## 4. Performance-Probleme

**Wichtig:** Ich habe kein Profiling und keine Benchmarks ausgeführt. Die Benchmarks in `src/benchmarks.rs` sind `#[ignore]` und geben nur bei manuellem Lauf Zeiten aus; `docs/TESTING.md`/`docs/COMPLETION-REPORT.md` enthalten manuelle Einmal-Messungen, die nicht CI-reproduzierbar sind. Nachfolgend sind **statisch belegte Risiken** und **unklärte Hypothesen** getrennt.

### Statisch belegt

- **Synchrones Schreiben im UI-Thread:** Jede Einstellungsänderung ruft `Library::save()` (`src/main.rs:412-452`, `src/ui/sidebar.rs:36`, `src/ui/header.rs`, `src/settings.rs:157`, `src/ui/dialogs.rs:46`) mit `create_dir_all`, `sync_all`, `persist` auf (`src/library.rs:497-505`). Lautstärke ist mit 300 ms entprellt (`src/main.rs:454-490`), anderes nicht.
- **`record_play()` schreibt bei jedem gezählten Play das gesamte Listening-JSON** (`src/main.rs:813-831`; bis 1000 History-Einträge + Maps, `src/listening.rs:72-99`).
- **`save_playback()` schreibt bei jedem Trackwechsel/Exit** ein vollständiges tempfile+fsync+rename für ein kleines Struct (`src/main.rs:541-571`, `src/gapless_ui.rs:124,142,160`, `src/main.rs:1258,1299,1338,1403,1579`).
- **`rebuild_locations()` ist O(n) über alle Playlists** und wird bei vielen Mutationen einzeln ausgelöst; Favorites-Entfernen ruft `edit_track()` pro Duplikat auf, jeweils mit vollem Rebuild und Save (`src/library.rs:358-385`, `src/main.rs:807-809,871-873`).
- **Track-Anzeige liest Tags synchron — behoben.** `show_track()` nutzt jetzt `Media::placeholder()` und einen Worker (`media_job`); `finish_media_job()` übernimmt das Ergebnis mit Generationsprüfung. Der Properties-Dialog nutzt `property_jobs` mit `loading`-Flag. Import/Export war bereits auf `mpsc` umgestellt. Messung in `docs/BENCHMARK-REPORT.md`: ~165 µs pro `Media::read` gegen ~0,16 µs pro `Media::placeholder`.
- **Such- und Sortier-Keys werden neu erzeugt:** `PlaylistView::build()` erzeugt pro Track einen lowercase-Key (`src/library.rs:199-217`), `SearchResults::new()` scannt Rows (`src/main.rs:223-260`); der 80-ms-Debounce begrenzt, aber nicht die Arbeit pro Suche.
- **Cache-Hits klonen `TrackInfo`** (`src/metadata_cache.rs:58,63,78-81`) und `Listening::rankings()` klont alle Keys (`src/listening.rs:42-61`).
- **Audio-Hot-Loop:** Mutex + `Next`-Clone pro Iteration (`src/audio/mod.rs:467,477`); TPDF-RNG zweimal pro Sample (`src/audio/shared.rs:197-204`, `src/audio/exclusive/render.rs:179-185`); FIR bis 2048 Taps pro Kanal/Frame (`src/audio/dsp.rs:5,240-252`); Resampler 256-Tap/256x (`src/audio/convert.rs:34-45`).
- **Ringpuffer bis ca. 8 MiB pro `pump()`** (`src/audio/mod.rs:416-418`) plus behaltene `pending`/`converted`-Kapazität (:454-457).
- **Geräteenumeration alle 1,5 s** mit neuem `Vec` + Sort, unabhängig von Änderungen (`src/audio/mod.rs:685-694`, `src/audio/shared.rs:31-49`).
- **Synchronous `AudioTrack::open()`** probt und dekodiert den ersten Packet sowie Lofty-Tags auf dem UI-Thread (`src/audio/mod.rs:176-177`, `src/audio/decoder.rs:143-155,194`).

### Hypothesen aus der Aufgabenstellung — im Audit **nicht bestätigt**

- „`taskbar_buttons()` wird mehrfach pro Frame berechnet“: Im Code wurde kein solcher Funktionsaufruf gefunden; `src/taskbar.rs` stellt einen Command-Queue/Callback bereit (:131-137, :239-288). **Nicht gemessen, nicht verifiziert.**
- „`current_is_favorite()` macht unnötige lineare Suche“: `Vec::contains` ist linear (`src/ui/header.rs:222-227`), aber ein Cache existiert bereits (:217-220). **Aufruffrequenz nicht gemessen.**
- „`SearchResults` berechnet Suchstrings unnötig oft neu“: Debounce und vorbereiteter `search_key` sind vorhanden (`src/main.rs:216-261`, `src/library.rs:102-110`, Commit `a5cfdbf`). **Kein Messbeleg für erneute Berechnung pro Frame.**

---

## 5. Risiken

- **Datenverlust/State-Inkonsistenz:** Directory-Fsync ist nachgeholt; offen bleiben keine Cross-File-Transaktion, Backup-Flag-Löschung vor Save-Erfolg und stille Corrupt-Behandlung bei `playback.json` und Cache.
- **Playback-Fehlsteuerung nach Edit:** indexbasierte Teilmodelle neben `TrackId`, fragiler Cursor-Rebase, verwaiste Queue-IDs.
- **Nicht deterministische Audio-Zustände:** Seek-Race, unbegrenztes Gate, stille Fehler in Render-Callbacks; `bit-perfect`-Anzeige ist ein Implementierungsstatus, kein hardware-verifiziertes Ergebnis.
- **Unsafe-Code-Fläche:** WASAPI/COM/Taskbar/WinRT nutzen `unsafe` mit均-Begründungen (`src/audio/exclusive/device.rs:37-417`, `src/audio/exclusive/render.rs:26-239`, `src/taskbar.rs:94-506`, `src/media_keys.rs:53-179`); Korrektheit hängt von COM-/WASAPI-Verträgen ab.
- **Git-Prozess:** gemeinsamer Git-Dir über 7 Worktrees, Branch ohne Upstream, lokales `main` 12 Commits hinter Origin, 6 unpushed Commits, Stash `stash@{0}`, geteilter Branch mit `development/library-scale-v06`. Höchste Vorsicht bei Reset/Checkout/Commit.
- **Artefakte:** `target/` ~3,20 GB (ignoriert), `android/` ~161 MB (9 Quelldateien untracked, Build-Outputs ignoriert). `lastfm-queue.json` und `*.invalid-*.json` sind **nicht** ignoriert und könnten versehentlich gestaget werden.
- **Secrets:** Keine hartcodierten API-Keys/Tokens/Zertifikate gefunden; Last.fm-Credentials zur Laufzeit via Credential Manager; `.env` ignoriert; Pages-Deploy nutzt OIDC. `android/local.properties` (ignoriert) enthält einen maschinenspezifischen SDK-Pfad.
- **Release/Doku:** Keine v0.4.0/v0.5.0-Tags, kein Release-Workflow trotz README-Beschreibung; veralteter DRAFT-Link; neue Backups undocumented.
- **Skalierbarkeit:** Alle Listenoperationen und Saves sind synchron; die Skalierungsbenchmarks sind ignoriert, es gibt also keinen automatisierten Wächter für 10k/50k/100k Titel.

---

## 6. Empfohlene Reihenfolge der Verbesserungen

Die Reihenfolge priorisiert **Datensicherheit vor Verhalten vor Performance** und berücksichtigt, dass Phasen 4–7 bereits teilweise umgesetzt sind. Jeder Schritt braucht Freigabe, bevor Code verändert wird; keine Schrittfolge schließt Git-Push ein.

1. **Arbeitsstand absichern (nur nach deiner Freigabe):** uncommittete Backup-Änderungen sichten, `PROJECT-AUDIT.md` und ggf. die Änderungen lokal committen (kein Push), `.gitattributes` für LF/CRLF und `.gitignore` für `*.invalid-*.json`/`lastfm-queue.json` ergänzen.
2. **Phase 3 abschließen — Persistenz härten:** Directory-Fsync nach `persist`, Backup-Flag erst nach erfolgreichem Save löschen, `previously_saved()` auf dieselbe Fehlerbehandlung wie `restore_playback()` heben, Tests für Round-Trip, Schreibfehler, vorherige Datei bleibt erhalten, fehlendes Verzeichnis, beschädigtes JSON, `playback.json` (inkl. `null`) und Listening-Limits.
3. **Metadaten-Cache robust machen:** Backup/Version/Reparatur- statt Stillschweigen, `expect`/`unwrap` im Indexer durch Fehlerfluss ersetzen, CUE-Abhängigkeit an den Stamp der Audiodatei koppeln, Test für „fehlender Cache baut sich neu auf“ und parallele Indexierung.
4. **Identität zentralisieren:** `Track`/Index-Paare schrittweise auf `TrackId` umstellen, `Selected`/`views`/`indexing` auf ID oder stabilem Playlist-Schlüssel, `refresh_playback_ids()`-Cursor auf „nächster statt vorheriger“ korrigieren und mit Test sichern; alte `playback.json` weiterhin über `SavedTrack::resolve` lesen.
5. **Favorites sauberstellen:** Membership konsistent (ID statt Pfad), `toggle_favorite()`-Tests (add/remove/save/load/play/sort/select), keine Löschung von Musikdateien, Sicherstellen dass keine Ordner entstehen; `current_is_favorite()`-Cache evaluieren.
6. **UI-Thread entlasten — umgesetzt:** `show_track()`/Properties über `media_job`/`property_jobs`, Import/Export über `mpsc`, `Library::save()` über den `SaveWorker`; Vorher/nachher-Messungen für Persistenz und Trackwechsel in `docs/BENCHMARK-REPORT.md`. Offen bleibt die Audio-Hot-Loop (siehe Nr. 7).
7. **Audio-Hot-Loop — teilweise umgesetzt:** `Next`-Clone/Mutex aus der Pump-Iteration entfernt (Epoch-Fast-Path) und Startup-Gate mit Timeout versehen. Seek-Handshake und `compatible()` bleiben bewusst unverändert; Audioqualität nicht verändern. Hörtest weiterhin offen.
8. **Beobachtbarkeit schaffen:** Reproduzierbare Benchmarks ( statt `#[ignore]`-Prints), Profiling/CPU/RAM/Frame-Time mit dokumentierter Umgebung, `BENCHMARK-REPORT.md` mit Rohdaten; erst danach gezielt optimieren.
9. **CI/Doku/Release-Hygiene:** Toolchain pinnen, DRAFT-Link korrigieren, Backup-Verhalten dokumentieren, Release-Tags/Workflow klären, Android-Artefakte aus dem Hauptworktree trennen oder bewusst tracken.

**Nicht empfohlen ohne weitere Freigabe:** Git Push/Merge/Release, Änderungen an `src/audio/` über den freigegebenen Umfang hinaus, automatische Löschung oder Verschiebung von Musikdateien, Änderungen an unabhängigen UI-Funktionen im Rahmen eines anderen Arbeitspakets.

---

## 7. Umsetzungsstand nach Welle 1 und den Audio-Quick-Wins

**Freigegeben und umgesetzt:**

- Resampler-Qualität wählbar (`Fast`/`Balanced`/`High`/`Maximum`), Standard `High` entspricht exakt dem alten Verhalten.
- FIR-Limit von 2.048 auf 16.384 Taps erhöht, README und Release Notes aktualisiert.
- Externe CUE-Dateien mit mehreren `FILE`-Einträgen; der letzte Track eines FILE läuft bis zum Dateiende. Eingebettete CUE-Daten in FLAC bleiben außen vor (lofty liefert dort keine Titel).
- ReplayGain-Scan über `bs1770` (ITU-R BS.1770-4) mit Schreiben von `REPLAYGAIN_TRACK_GAIN`/`REPLAYGAIN_TRACK_PEAK` aus den Track-Eigenschaften. Der Scan läuft in einem Hintergrund-Thread; `finish_gain_jobs()` nimmt das Ergebnis per Kanal entgegen und aktualisiert Ansichten und Anzeige. Damit ist der in Nr. 6 genannte UI-Blocking-Punkt für diesen Pfad behoben.
- Einfacher Tag-Editor für Titel, Interpret, Album, Album-Interpret und Tracknummer; leere Felder entfernen den Wert, CUE-Einträge verweisen auf ihre Quelldatei.
- `.gitattributes`, `.gitignore`-Einträge, Toolchain-Pin `1.100.0`, CI-Release-Build, korrekter Release-Notes-Link und dokumentiertes Backup-Verhalten.
- Reproduzierbare Benchmarks mit CSV-Rohdaten (`target/benchmark-*.csv`) und Auswertung in `docs/BENCHMARK-REPORT.md`.
- UI-Thread-Offload für den ReplayGain-Scan (`finish_gain_jobs()` mit Kanal), gemeinsame `refresh_metadata()` für Tag- und Gain-Änderungen.
- Startup-Gate im Decoder-Worker mit 5-Sekunden-Timeout und Warnung statt endlosem Warten (`wait_for_gate()`, mit Test).
- Hintergrund-Schreiber (`src/save_worker.rs`) für `library.json`, `playback.json` und `listening.json`: UI serialisiert, Worker macht `fsync`/`persist` und coalescht pro Datei. `shutdown()` joint beim Exit, daher geht keine Änderung verloren. Vier Tests für Coalescing, Reihenfolge, getrennte Ziele und Fehlermeldungen.
- Playlist-Import und -Export laufen über `mpsc` in Worker-Threads; `finish_playlist_io()` übernimmt Ergebnis und Status im UI-Thread (gleiches Muster wie der vorhandene Audio-Import).
- Vorher/nachher-Messung des Persistenzpfads in `docs/BENCHMARK-REPORT.md` mit Rohdaten in `target/benchmark-save.csv`.
- Directory-Fsync nach jedem `persist()` (`sync_directory()` in `src/library.rs`): auf Windows mit `FILE_FLAG_BACKUP_SEMANTICS` + `GENERIC_WRITE`, weil `FlushFileBuffers` Schreibzugriff verlangt; auf Unix über `sync_all()` auf dem geöffneten Verzeichnis. Damit ist der Rename selbst stromausfallsicher, nicht nur der Dateiinhalt.
- GitHub-Button unter **Tools** im Header und unter **Settings → About**; beide öffnen `https://github.com/AikasAcc/rust-player`. `PROJECT_URL` liegt zentral in `src/main.rs`.
- `show_track()` und der Properties-Dialog lesen Tags nicht mehr synchron: `Media::placeholder()` bzw. `TrackProperties::loading()` erscheinen sofort, `media_job`/`property_jobs` liefern das Ergebnis per `mpsc` nach. `media_generation` verwirft veraltete Ergebnisse bei schnellem Trackwechsel; `loading` schützt Nutzeränderungen im Dialog vor späten Worker-Ergebnissen. Cover-Textur, Lyrics und Plugin/Media-Key-Benachrichtigungen laufen erst nach dem vollständigen Lesen. Messung: `docs/BENCHMARK-REPORT.md`, Rohdaten `target/benchmark-media.csv`.
- Audio-Hot-Loop: `Next`-Clone und `Mutex` pro Pump-Iteration durch `next_epoch: AtomicU64` + `changed_next()` ersetzt. Der Mutex bleibt nur für echte Änderungen; `queue_next()` publisht mit `Release`, der Pump liest mit `Acquire`. Test `queue_changes_are_observed_once_without_polling_the_mutex` prüft Cache, identische Re-Queues und None-Übergang. Kein Hörtest durchgeführt.
- Zusätzliche Symphonia-Codecs/-Container: MP1, MP2, ADPCM (MS und IMA in WAV/CAF) und MKV/MKA. `audio_track()` bevorzugt dekodierbare Spuren, damit Videospuren in MKV die Wiedergabe nicht blockieren. Test `extra_codecs_are_compiled_in` prüft die Decoder-Registry. `Cargo.lock` um `symphonia-codec-adpcm` und `symphonia-format-mkv` ergänzt.

**Bewusst nicht geändert:**

- `compatible()` in `src/audio/gapless.rs`: `SourceFormat.kind` kodiert die Bit-Tiefe bereits als `Integer(bits)`; der Codec ist für die PCM-Fortsetzung irrelevant, und `gain` wird pro Source über `converter.set_gain()` übernommen. Eine Ergänzung würde die Gapless-Erkennung eher verschlechtern als verbessern.
- `Next`-Clone/Mutex in der Pump-Iteration: der Umbau berührt die gapless Übergabekette und damit die Audioqualität. Ohne separates Profiling und Hörtest ist die Gefahr eines Rückschritts größer als der vermutete Gewinn.
- Backup-Rotation: Backups werden absichtlich aufgehoben, damit beschädigte Originale untersuchbar bleiben; das Fehlen einer automatischen Rotation ist dokumentiert statt stillschweigend geändert.
- `android/` bleibt untracked und ist nicht Teil dieser Arbeit; es wurde weder gelöscht noch in die Versionsverwaltung aufgenommen.

**Umgesetzt aus Nr. 8/9:** reproduzierbare Benchmarks schreiben CSV nach `target/benchmark-*.csv`, Auswertung in `docs/BENCHMARK-REPORT.md`, Toolchain-Pin, CI-Build, DRAFT-Link und Backup-Doku.

**Offen aus Nr. 6–9:** Hörtest für den Audio-Hot-Loop (Änderung ist umgesetzt und logisch getestet, aber nicht gehört); Seek-Handshake und `compatible()` bewusst unverändert; Release-Tags/Workflow-Klärung; Trennung oder bewusste Trackung der untracked `android/`-Artefakte. Der in Nr. 6 genannte Offload für `show_track()`/Track-Properties, Import/Export und `Library::save()` ist dagegen vollständig umgesetzt und messbar belegt.
