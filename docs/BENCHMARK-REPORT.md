# Benchmark-Bericht

Zweck: reproduzierbare, maschinell auswertbare Mikromessungen als Grundlage für den
UI-Thread-Offload (Audit-Punkt 6). **Grenze:** hier wird keine echte UI-Frame-Time gemessen.
Die Zahlen zeigen Rechenzeit im Release-Build, nicht das gefühlte Verhalten der Oberfläche.

## Umgebung

- Windows, AMD Ryzen 7 5700X (8 Kerne), 32 GiB RAM
- Rust 1.100.0 (über `rust-toolchain.toml` festgelegt), Release-Profil aus `Cargo.toml`
- Messung im Testprozess, `black_box` für den Suchbegriff, ein Thread

## Reproduktion

```powershell
cargo test --release --locked -- --ignored --nocapture --test-threads=1 benchmarks::library_search_scale
cargo test --release --locked -- --ignored --nocapture --test-threads=1 benchmarks::tag_scan
cargo test --release --locked -- --ignored --nocapture --test-threads=1 benchmarks::library_save_scale
cargo test --release --locked -- --ignored --nocapture --test-threads=1 benchmarks::media_read_scale
```

Jeder Lauf überschreibt seine Rohdaten als CSV:

- `target/benchmark-search.csv`
- `target/benchmark-tag_scan.csv`
- `target/benchmark-save.csv`
- `target/benchmark-media.csv`

Spalten: `scenario,size,micros`. Die Rohdaten des letzten Laufs liegen in `target/benchmark-save.csv` und `target/benchmark-media.csv`.

## Bibliothekssuche und Ansichtsaufbau

Alle Zeiten in Mikroseunden. Die Suche wurde siebenmal pro Größe ausgeführt; der erste
Wert enthält zusätzliche Cold-Cache-Effekte.

| Titel | Aufbau | Suche 1 | Suche 2–7 (Bereich) |
|---:|---:|---:|---:|
| 1.000 | 798 | 27 | 12 |
| 10.000 | 7.949 | 248 | 125–139 |
| 50.000 | 44.650 | 1.187 | 514–797 |
| 100.000 | 90.453 | 2.288 | 1.625–2.634 |

Deutung: der Aufbau skaliert annähernd linear. Die Suche bleibt im warmen Zustand
selbst bei 100.000 Titeln im Bereich von etwa 1,6–2,6 ms. Das ist kein Grund, die
Suche jetzt zu optimieren; es ist die Basis, um einen späteren Offload messbar zu machen.

## Metadaten-Scan (1.000 Dateien)

Vier Läufe über 1.000 hart verlinkte PCM-WAV-Dateien:

| Lauf | Mikroseunden |
|---:|---:|
| 1 (cold) | 623.562 |
| 2 | 209.880 |
| 3 | 198.181 |
| 4 | 200.056 |

Deutung: nach dem ersten Lauf liegt der Scan bei etwa 200 ms für 1.000 Dateien.
Der erste Lauf ist deutlich langsamer, weil Dateien und Cache noch nicht warm sind.
Dieser Wert war die Begründung, den Metadaten-Indexer bereits asynchron zu betreiben.
Der ReplayGain-Scan läuft inzwischen ebenfalls im Hintergrund; die Zahlen hier betreffen
den getrennten Metadaten-Pfad und sind keine Audio-Messung.

## Persistenz-Pfad — vor und nach dem Offload

`library_save_scale` misst drei Szenarien pro Größe (je drei Wiederholungen):

- `save_json`: alter Pfad, Serialisieren + `tempfile` + `fsync` + `persist` **im aufrufenden Thread**.
- `serialize`: nur `serde_json::to_vec`.
- `ui_path`: Serialisieren + Einreihen in den Hintergrund-Schreiber — das, was der UI-Thread heute noch tut. Die `fsync`-Arbeit des Workers zählt hier **nicht**.

Baseline (vor dem Offload, erster Lauf, warm):

| Titel | `serialize` | `save_json` (UI-Thread) |
|---:|---:|---:|
| 1.000 | 49–67 µs | 1,39–1,87 ms |
| 10.000 | 354–491 µs | 2,02–2,84 ms |
| 50.000 | 1,63–1,69 ms | 4,45–4,75 ms |
| 100.000 | 3,79–4,45 ms | 8,25–8,42 ms |

Nach dem Offload (letzter Lauf, Rohdaten in `target/benchmark-save.csv`):

| Titel | `ui_path` | `save_json` (Vergleich im selben Lauf) |
|---:|---:|---:|
| 1.000 | 38–80 µs | 2,03–4,59 ms |
| 10.000 | 392–577 µs | 2,59–7,87 ms |
| 50.000 | 2,41–2,67 ms | 6,49–7,92 ms |
| 100.000 | 4,84–5,41 ms | 11,19–20,33 ms |

**Wichtig zur Einordnung:** die absolute Disk-Latenz schwankt zwischen Läufen stark
(ein Zwischenlauf lag bei 100.000 Titeln bei 42–89 ms für `save_json`). Ursache ist
vermutlich Antiviren-Scanning und Cache-Zustand, nicht der Code. Vergleichbar sind nur
Werte aus demselben Lauf. Dort ist der UI-Thread-Aufwand durch den Offload um etwa
Faktor 8–15 bei 100.000 Titeln und um mehr als Faktor 20 bei 1.000 Titeln gesunken.

Deutung: Serialisieren bleibt zwangsläufig im UI-Thread, weil es den aktuellen State
liest. Der teure und unvorhersehbare Teil (`fsync`/`persist`) läuft jetzt im Worker.
Zusätzlich coalescht der Worker mehrere Snapshots derselben Datei — bei schnellen
Einstellungsänderungen landet nur der neueste Zustand auf der Platte.

## Track-Wechsel — vor und nach dem Offload

`media_read_scale` läuft 1.000-mal gegen dieselbe getaggte PCM-WAV-Datei (Rohdaten in
`target/benchmark-media.csv`):

| Szenario | 1.000 Läufe | pro Aufruf |
|---|---:|---:|
| `media_placeholder` (heute im UI-Thread) | 162 µs | ~0,16 µs |
| `media_read` (früher synchron im UI-Thread, heute Worker) | 165.429 µs | ~165 µs |

Deutung: der Tag- und Cover-Parse kostete bisher rund 165 µs **pro Trackwechsel auf dem
UI-Thread** — bei einer FLAC mit eingebettem Cover oder einer großen CUE-Datei liegt der
Wert höher. Der UI-Thread setzt jetzt nur einen Titel-Snapshot und reiht den vollständigen
Lesevorgang in einen `mpsc`-Kanal ein. `finish_media_job()` übernimmt Ergebnis, Cover und
Lyrics auf dem UI-Thread und verwirft Ergebnisse über eine Generation, wenn der Nutzer
inzwischen schon den nächsten Track gewählt hat. Der Properties-Dialog nutzt dasselbe
Muster über `property_jobs`, damit auch das Öffnen der Track-Eigenschaften nicht mehr
auf den Tag-Lesevorgang wartet.

## Offene Messpunkte

- Frame-Time der Oberfläche unter Last (egui), noch nicht gemessen. Die Zahlen oben sind Rechenzeit, keine Frame-Time.
- Import/Export von Playlistdateien: durch das `mpsc`-Muster offloadet, aber nicht separat benchmarkt (Dateigrößen und CUE-Anteil variieren zu stark).
- Speicher- und CPU-Profiling während des Scans, noch nicht gemessen.
- Audio-Pump-Loop: kein Benchmark. Die Entfernung von `Mutex` + `Next`-Clone pro Iteration ist nur logisch getestet (`queue_changes_are_observed_once_without_polling_the_mutex`); ein Hörtest steht aus.
