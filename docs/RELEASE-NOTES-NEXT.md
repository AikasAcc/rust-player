# Rust Player – Änderungen der nächsten Version

Diese Version macht Bibliothek, Favoriten und Wiedergabe deutlich robuster.
Die Bedienung wird um zwei kleine Aktionen ergänzt. Deine Dateien bleiben, wo sie sind.

---

## Kurzfassung (für die Release-Seite)

- **Favoriten bleiben bei der richtigen Datei** – auch nach Umsortieren, Umbenennen oder Verschieben von Songs.
- **Der zuletzt gespielte Song startet zuverlässig wieder** – an der richtigen Stelle, auch nach Änderungen an Playlists.
- **„Weiter" trifft wieder den richtigen Titel** – besonders nach dem Löschen des gerade laufenden Songs.
- **Doppelte Dateien bleiben getrennt** – dieselbe Datei in zwei Playlists bleibt zwei eigenständige Einträge.
- **Keine Datenverluste bei beschädigten Einstellungen** – unlesbare Dateien werden gesichert, nicht überschrieben.
- **Alte Daten werden automatisch übernommen** – vorhandene `library.json` und `playback.json` funktionieren weiter.
- **Bibliothek startet schneller** – die Metadaten-Ablage wird nur noch bei echten Änderungen erneuert und regelmäßig aufgeräumt.
- **Kein neuer Favoriten-Ordner** – Favoriten bleiben eine Playlist; Originaldateien werden weder kopiert noch verschoben.
- **Hörbarkeit einstellen** – vier Resampler-Qualitätsstufen statt einer festen Einstellung; die alte Klangqualität bleibt der Standard.
- **Längere FIR-Filter möglich** – jetzt 16.384 statt 2.048 Taps pro Kanal.
- **ReplayGain selbst schreiben** – ein Klick in den Tile-Eigenschaften misst die Datei und speichert die Tags.
- **Tags direkt bearbeiten** – Titel, Interpret, Album, Album-Interpret und Tracknummer lassen sich ändern.
- **CUE mit mehreren Dateien** – Alben mit einer Audiodatei pro Track werden jetzt ebenfalls gelesen.
- **Herz direkt beim Song** – Favoriten lassen sich jetzt in jeder Playlist zeilenweise setzen und entfernen.
- **Playlist per Doppelklick starten** – ein Doppelklick auf eine Playlist spielt sie ab; bei aktivem Shuffle beginnt ein zufälliger Titel.
- **Oberfläche bleibt beim Speichern flüssig** – das Schreiben der Einstellungen läuft im Hintergrund; beim Beenden wird alles fertig geschrieben.
- **Import und Export ohne Rucken** – Playlistdateien werden im Hintergrund gelesen und geschrieben.

---

## Ausführliche Fassung (für Endnutzer)

### Favoriten

Favoriten sind jetzt fest mit dem jeweiligen Song verbunden – nicht mehr nur mit seinem Dateipfad.

Was du davon merkst:

- Du kannst Songs in einer Playlist umsortieren, umbenennen oder auf der Festplatte verschieben: Der Favoriten-Stern bleibt beim richtigen Song.
- Wenn du einen Song aus einer anderen Playlist hinzufügst, bleibt die Verbindung zum Original erhalten. Wird der Original-Song entfernt, verschwindet auch der veraltete Favorit – statt als „Geist" stehen zu bleiben.
- Wird ein Song aus den Favoriten direkt entfernt oder verschoben, stimmt die Zuordnung weiterhin.
- Der Favoriten-Stern in der Kopfzeile zeigt auch dann den richtigen Zustand, wenn du gerade direkt aus der Favoriten-Playlist abspielst.
- Jede Song-Zeile hat jetzt ein eigenes Herz zwischen Nummer und Titel. Ein Klick darauf setzt oder entfernt den Favoriten – auch direkt in der Favoriten-Playlist. Ein gefülltes Herz ist rot, ein leeres ausgegraut.

Wichtig: Es wird **kein Ordner** „Favorites" angelegt. Favoriten bleiben eine normale Playlist in `library.json`. Deine Musikdateien werden weder kopiert noch verschoben.

### Wiedergabe und Warteschlange

- Beim Start merkt sich der Player den zuletzt gespielten Song und die Position. Das funktioniert jetzt auch dann zuverlässig, wenn sich Playlists zwischenzeitlich verändert haben.
- Alte gespeicherte Wiedergabedaten ohne die neue Zuordnung werden beim ersten Laden automatisch übernommen und aktualisiert.
- Wird der gerade laufende Song aus einer Playlist gelöscht, springt „Weiter" auf den tatsächlich nächsten Titel – nicht mehr versehentlich auf den vorherigen.
- Die Warteschlange („Als Nächstes") bleibt auch nach dem Entfernen oder Verschieben von Songs stimmig.
- Ein Doppelklick auf eine Playlist in der linken Leiste – oder auf „Favorites" – startet die Wiedergabe dieser Playlist. Ist Shuffle an, beginnt ein zufälliger Titel; sonst der erste Titel in der aktuellen Sortierung. Ein einzelner Klick wählt die Playlist weiterhin nur aus.

### Playlists und Dateien

- Dieselbe Datei kann weiterhin in mehreren Playlists liegen. Jeder Eintrag bleibt eigenständig und wird nicht mit dem anderen „vermischt".
- Duplikate mit gleichem Dateinamen in verschiedenen Ordnern bleiben getrennt.
- CUE-Tracks werden weiterhin als eigene Einträge behandelt.
- Unter Windows werden Pfade unabhängig von Groß-/Kleinschreibung und von `/` bzw. `\` erkannt. Ein umbenannter Ordner führt also nicht mehr zu „verschwundenen" Favoriten oder falsch zugeordneten Einträgen.

### Sicherer Umgang mit deinen Daten

- Einstellungen, Bibliothek, Hörverlauf und Wiedergabestand werden jetzt sicher gespeichert: Erst wenn die neue Datei vollständig geschrieben ist, ersetzt sie die alte. Ein Absturz mittendrin beschädigt die bisherigen Daten nicht.
- Ist eine Datei trotzdem unlesbar, legt der Player automatisch eine Sicherungskopie an (z. B. `library.invalid-…`) und baut die Daten neu auf. Die beschädigte Datei wird nicht stillschweigend überschrieben.
- Ältere oder unvollständige Dateien werden weiterhin geladen und mit sinnvollen Standardwerten ergänzt.
- Das Schreiben läuft jetzt im Hintergrund, bleibt aber atomar. Beim ordnungsgemäßen Beenden wartet der Player, bis alle Änderungen auf der Platte liegen. Schreibfehler werden in der Statuszeile gemeldet.
- Nach dem Ersetzen der Datei wird zusätzlich das Verzeichnis synchronisiert. Damit bleibt auch bei einem Stromausfall die neue Datei erhalten und nicht die alte oder gar keine.

### Metadaten und CUE

- Die zwischengespeicherten Track-Informationen werden nur noch dann neu gelesen, wenn sich die Datei tatsächlich geändert hat. Das macht den Start und das Durchsuchen schneller.
- Bei CUE-Dateien wird sowohl die CUE-Liste als auch die zugehörige Audiodatei geprüft: Ändert sich eine davon, werden die Track-Infos korrekt aktualisiert.
- Entfernte Songs und Playlists hinterlassen keine veralteten Einträge mehr im Zwischenspeicher.
- Ein beschädigter oder von einer neueren Version erzeugter Zwischenspeicher wird gesichert und sauber neu aufgebaut, statt falsch interpretiert zu werden.
- Beim Trackwechsel erscheint Titel und Oberfläche sofort; Tags, eingebettetes Cover und Lyrics werden im Hintergrund nachgeladen. Das Anzeigefeld bleibt auch beim schnellen Überspringen mehrerer Titel konsistent.
- Die Titel-Eigenschaften öffnen sofort und zeigen „Reading tags…", bis die vollständigen Angaben da sind. Änderungen, die du in der Zwischenzeit machst, werden nicht durch das nachträgliche Laden überschrieben.

### Audio und Tags

- Unter **Einstellungen → Audio** gibt es jetzt vier Resampler-Stufen: Schnell, Ausgewogen, Hoch und Maximal. „Hoch" entspricht genau der bisherigen Einstellung, wer also nichts umstellt, hört keinen Unterschied.
- FIR-Korrekturdateien dürfen jetzt bis zu 16.384 Taps pro Kanal enthalten (vorher 2.048). Bei 48 kHz sind das gut 340 ms.
- In den Titel-Eigenschaften kannst du mit „ReplayGain scannen und schreiben" die Lautstärke nach ITU-R BS.1770-4 messen und `REPLAYGAIN_TRACK_GAIN` sowie `REPLAYGAIN_TRACK_PEAK` in die Datei schreiben. Es werden nur Tags geändert; die Musik selbst bleibt unberührt. Der Scan läuft im Hintergrund, die Oberfläche bleibt bedienbar und die Wiedergabe läuft weiter.
- Ebenfalls in den Titel-Eigenschaften lassen sich Titel, Interpret, Album, Album-Interpret und Tracknummer ändern und mit „Tags speichern" zurückschreiben. Leere Felder entfernen den alten Wert. Bei CUE-Einträgen wird stattdessen die zugehörige Audiodatei bearbeitet.
- Externe CUE-Dateien dürfen jetzt auf mehrere Audiodateien verweisen. Der letzte Titel einer Datei läuft bis zu deren Ende.
- Neue Dateiformate: MP1 und MP2 (MPEG Layer I/II), Audio in MKA/MKV sowie ADPCM in WAV und CAF. Bei MKV-Dateien mit Videospur wird die erste dekodierbare Audiospur gewählt. WavPack, Monkey's Audio und DSD bleiben weiterhin nicht unterstützt.
- Der Audio-Worker fragt die nächste Wiedergabe nicht mehr bei jeder Iteration über eine Sperre ab und klont sie nicht mehr. Er merkt sich den letzten Stand und reagiert nur auf tatsächliche Änderungen. Das reduziert Sperren und Speicherzuweisungen im Echtzeitpfad; der Klang selbst bleibt unverändert.

### Kompatibilität

- Vorhandene `library.json`, `playback.json` und der Hörverlauf bleiben gültig und werden automatisch weiterverwendet.
- Es sind keine manuellen Schritte nötig; ein Backup deiner Einstellungen ist trotzdem nie verkehrt.
- Die Tastenkürzel und die grundlegende Bedienung bleiben unverändert; neu sind nur das Herz pro Zeile und der Doppelklick auf Playlists.
- Der GitHub-Button ist wieder da: unter **Tools** im Kopf sowie unter **Einstellungen → About**. Beide öffnen das Projektrepository.

---

## Für Neugierige (kurz, technisch)

- Songs erhalten eine stabile, zufällige Kennung pro Playlist-Eintrag. Diese Kennung – nicht der Pfad – bestimmt, welcher Song gemeint ist.
- Favoriten speichern zusätzlich die Kennung des Original-Songs. Der Pfad bleibt nur als Anzeige und als Fallback für alte Daten erhalten.
- Wiedergabestand und Warteschlange arbeiten mit denselben Kennungen; Index-Positionen werden daraus abgeleitet.
- Speichern erfolgt atomar über eine temporäre Datei; beschädigte JSON-Dateien werden mit Präfix gesichert. `sync_directory()` flusht nach dem Rename auch den Verzeichniseintrag: unter Windows via `CreateFileW` mit `FILE_FLAG_BACKUP_SEMANTICS` und `GENERIC_WRITE` (nur ein lesbares Handle würde `FlushFileBuffers` verweigern), auf Unix via `sync_all()` auf dem geöffneten Verzeichnis.
- Der Metadaten-Zwischenspeicher ist versioniert und wird bei Bedarf migriert, gesichert oder neu aufgebaut.
- Der Resampler nutzt 64/128/256/512 Taps je nach Stufe; der Standard bleibt bei 256 Taps mit kubischer Interpolation.
- Die Lautstärkemessung läuft über `bs1770` (ITU-R BS.1770-4), schreibt ReplayGain-2.0-Referenz (-18 LUFS) und den Sample-Peak; CUE-Segmente werden über die vorhandenen Track-Grenzen gemessen.
- Tag-Änderungen werden über lofty in die Originaldatei geschrieben und anschließend über den bestehenden Metadaten-Indexer in allen Playlist-Ansichten aktualisiert.
- Der ReplayGain-Scan läuft in einem Hintergrund-Thread; das Ergebnis wird über einen Kanal in der Oberfläche übernommen.
- Ein hängender Gapless-Übergabeschalter gibt den Decoder nach fünf Sekunden frei und meldet eine Warnung, statt den Worker dauerhaft zu blockieren.
- `RenderState` hält `next` weiter in einem `Mutex`, aber zusätzlich einen `next_epoch: AtomicU64`. `queue_next()` erhöht den Zähler nur bei echter Änderung; `changed_next()` liefert dem Pump-Thread nur bei Zählerwechsel ein geklontes `Next`. Der Echtzeitpfad macht damit statt Lock+Clone pro Iteration einen Atomic-Load. Ein Test prüft, dass identische Anfragen keinen Wechsel auslösen.
- Symphonia-Features `mp1`, `mp2`, `adpcm` und `mkv` sind aktiv; `audio_track()` bevorzugt bei MKV einen tatsächlich dekodierbaren Track statt der ersten nicht-leeren Spur. `Cargo.lock` enthält die beiden neuen Symphonia-Crates.
- Das Herz in der Zeile nutzt dieselbe stabile Track-Kennung wie die Kopfzeile; `toggle_favorite_id()` ist damit für beide Wege gemeinsam zuständig.
- Der Playlist-Start wählt bei Shuffle einen Zufallsindex und sonst den ersten Eintrag der sichtbaren Sortierreihenfolge (`playlist_entry()`); dieselbe Logik trägt auch den regulären Start über „Play".
- `library.json`, `playback.json` und `listening.json` werden weiterhin atomar geschrieben (temporäre Datei + `fsync` + Ersetzen), aber von einem Hintergrund-Thread. Der UI-Thread serialisiert nur noch das Snapshot; mehrere Snapshots derselben Datei werden zusammengefasst. Beim Beenden wartet die Oberfläche auf den Abschluss (`shutdown()`).
- Playlist-Import und -Export nutzen Worker-Threads mit `mpsc`; nur die Änderung der Bibliothek und die Statusmeldung passieren im UI-Thread.
- `Media::placeholder()` liefert nur den Dateititel (inkl. CUE-Auflösung); der vollständige `Media::read()` läuft in `media_job` auf einem Worker. `finish_media_job()` übernimmt Cover-Textur, Lyrics und die Benachrichtigung von Plugins/Media-Keys und startet danach Cover- und Lyrics-Suche. Eine Generation verwirft veraltete Ergebnisse bei schnellem Trackwechsel. `property_jobs` macht dasselbe für den Properties-Dialog; `loading` verhindert, dass ein späteres Worker-Ergebnis Nutzeränderungen überschreibt.
- Messwerte vor/nachher stehen in `docs/BENCHMARK-REPORT.md`. Bei 100.000 Titeln sinkt der Aufwand im UI-Thread von rund 8–20 ms auf rund 5 ms; die Schwankung der Festplattenlatenz wird im Bericht offen gelegt.
