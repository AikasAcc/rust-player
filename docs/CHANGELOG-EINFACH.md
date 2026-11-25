# Rust Player v0.6.0-rc1 – Was neu ist

Das ist eine **Vorschau** (Release Candidate). Die aktuelle stabile Version bleibt **v0.5.0**. Bitte melde Probleme, bevor daraus ein volles Release wird.

## Kurz gesagt

Der Player startet schneller, zeigt Titel und Cover ohne langes Warten, speichert deine Daten sicherer und spielt mehr Audio-Dateien ab. Der Klang selbst ist unverändert.

## Mehr Musikdateien funktionieren

Neu erkannt werden:

- **MP1 und MP2** – ältere MPEG-Audiodateien
- **ADPCM in WAV- und CAF-Dateien** – ältere Aufnahmen
- **MKA und MKV** – Audio-Container. Auch wenn eine Videospur drin steckt, findet der Player die Tonspur.

Weiterhin **nicht** unterstützt: WavPack, Monkey's Audio, DSD, ASIO, VST-Plugins und UPnP.

## Die Wiedergabe läuft ruhiger

- Der Player fragt nicht mehr bei jedem kleinen Arbeitsschritt nach, ob ein neuer Song ansteht. Das konnte bei starker Belastung zu kurzen Aussetzern führen.
- Bleibt ein Übergang zwischen zwei Songs hängen, gibt der Player nach fünf Sekunden auf und meldet das, statt für immer zu warten.
- **Der Klang ist unverändert** – es geht nur um Stabilität.

## Titel und Cover erscheinen sofort

- Früher musste erst die ganze Datei gelesen werden, bevor der Titel auftauchte. Jetzt siehst du den Titel sofort; Cover und Details kommen kurz danach.
- Wenn du schnell mehrere Songs überspringst, bleibt die Anzeige beim richtigen Song.
- Das Fenster **Titel-Eigenschaften** öffnet sofort. Was du inzwischen eintippst, wird nicht durch späte Worker-Ergebnisse überschrieben.

## Deine Daten sind besser geschützt

- Einstellungen, Playlists und Hörverlauf werden im Hintergrund gespeichert. Beim Schließen wartet der Player kurz, bis alles wirklich auf der Platte liegt.
- Nach dem Speichern wird auch der Ordner selbst abgesichert. Bei einem Stromausfall bleibt dadurch die neue Datei erhalten.
- Ist eine Datei beschädigt, legt der Player eine Sicherungskopie an und baut die Daten neu auf. Alte Sicherungen werden **nie automatisch gelöscht**.

## Kleine Verbesserungen in der Bedienung

- Herz in jeder Songzeile – Favoriten direkt in der Liste.
- Doppelklick auf eine Playlist oder auf Favoriten startet die Wiedergabe.
- GitHub-Button unter **Tools** und unter **Einstellungen → About**.

## Klang und Tags

- Unter **Einstellungen → Audio** gibt es vier Qualitätsstufen. **„Hoch“ ist unverändert** – wer nichts umstellt, hört keinen Unterschied.
- Korrekturdateien (FIR) dürfen doppelt so lang sein.
- **ReplayGain** kann im Hintergrund gemessen werden. Nur die Lautstärke-Information wird in die Datei geschrieben, die Musik bleibt unberührt.
- Im Titel-Fenster kannst du Titel, Interpret, Album, Album-Interpret und Tracknummer ändern.
- Externe CUE-Dateien dürfen auf mehrere Audiodateien verweisen.

## Das bleibt gleich

- Tastenkürzel, Playlists, Favoriten, Hörverlauf und Einstellungen bleiben gültig.
- Es werden keine Musikdateien verschoben, umbenannt oder gelöscht.
- Bestehende `library.json`, `playback.json` und Hörverlauf werden automatisch weiterverwendet.

## Bitte im Pre-Release prüfen

1. Zwei Songs gleicher Samplerate und Bit-Tiefe nacheinander: bleibt der Übergang ohne Klick?
2. Während der Wiedergabe vorwärts und rückwärts springen: keine hängenden oder falschen Positionen?
3. Eine `.mp1`/`.mp2`, eine ADPCM-WAV und eine `.mka` laden.

## Installation

ZIP herunterladen, entpacken und `Rust-Player.exe` starten. Es gibt keinen Installer. Die SHA-256-Prüfsumme liegt als `.sha256`-Datei daneben.
