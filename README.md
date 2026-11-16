# Screacher Radio

Kleiner Webradio-Player in Rust mit FLTK für Windows und Linux.

## Bedienung

- Unter **Hinzufügen…** einen Namen und eine HTTP(S)-URL angeben. Direkte MP3-Streams sowie M3U- und PLS-Playlists werden unterstützt. Eine Playlist erscheint als ein Sender und spielt ihren ersten Stream.
- Das Dreieck startet den ausgewählten Sender, das Quadrat stoppt die Wiedergabe. Beim Programmstart ist der zuletzt erfolgreich gespielte Sender vorausgewählt.
- Rechts neben der Senderliste stehen die Laufzeit seit Wiedergabestart und, wenn der Stream ICY-Metadaten liefert, Titel und Interpret. Ein Eintrag der Form `Interpret - Titel` wird in beide Felder aufgeteilt.
- Die Lautstärke lässt sich auch mit dem Mausrad über dem Regler ändern. Die horizontalen Balken **L** und **R** zeigen die Spitzenpegel mit einem Verlauf von Grün über Gelb zu Rot. Bei Mono zeigen beide denselben Pegel.
- Sender lassen sich löschen, über den Stern als Favoriten markieren und nach Name oder URL suchen. Ein voller Stern kennzeichnet einen Favoriten. **Nur Favoriten** lässt sich mit der Suche kombinieren.
- Das Uhrsymbol öffnet den **Sleeptimer**. Er stoppt die Wiedergabe nach 15, 30, 45 oder 60 Minuten oder nach einer frei eingegebenen Dauer. Einen laufenden Timer kannst du um 5, 10 oder 15 Minuten verlängern. Beim Schließen des Dialogs und im Tray läuft der Countdown weiter.
- Unter **Einstellungen…** findest du **Immer im Vordergrund**, die Themes **Standard** und **Dunkel** sowie das Minimieren in den Tray. Theme-Wechsel werden sofort sichtbar. **Esc** blendet bei aktiver Tray-Option und verfügbarem Tray das Hauptfenster aus. Das Tray-Icon öffnet den Player wieder. Unter Linux braucht der Desktop eine StatusNotifierItem-kompatible Tray-Leiste und für **Immer im Vordergrund** einen Fenstermanager mit EWMH-Unterstützung.

Der Player liest und speichert Sender und Favoriten in `stations.yaml` direkt neben der Programmdatei. Beim ersten Start ohne diese Datei übernimmt er vorhandene Sender aus `settings.json`. Zum Ändern der Senderliste muss der Programmordner beschreibbar sein. Die YAML-Datei kannst du mit der Programmdatei in einen anderen Ordner mitnehmen.

```yaml
- name: Mein Sender
  url: https://example.org/radio.mp3
  favorite: true
```

Letzter Sender und Lautstärke bleiben in `settings.json`. Unter Windows liegt die Datei in `%APPDATA%\Screacher Radio\config`, unter Linux in `${XDG_CONFIG_HOME:-$HOME/.config}/screacherradio`.

## Downloads aus GitHub Actions

Der Workflow [Build binaries](.github/workflows/build.yml) läuft bei einem Push oder per **Actions → Build binaries → Run workflow**. Nach erfolgreichem Build stehen im jeweiligen Lauf unter **Artifacts** die Windows-EXE und das Linux-Archiv `tar.gz` für x86_64 zum Download bereit. Der Linux-Build nutzt das Debian-Image aus dem Dockerfile. Die Laufzeitabhängigkeiten stehen unten im Abschnitt Linux.

## Windows

Voraussetzungen sind Rust (stable), CMake ab 3.28 und ein zum Rust-Target passender C++17-Compiler. Für `windows-msvc` eignen sich die Visual Studio Build Tools mit C++-Desktopentwicklung, für `windows-gnu` MinGW-w64 mit `gcc`, `g++` und `mingw32-make` im `PATH`.

```powershell
cargo run --locked
cargo build --release --locked
.\target\release\screacher-radio.exe
```

## Linux

Docker baut und testet das Programm in Debian Trixie und exportiert die Binärdatei. Das Ergebnis benötigt eine kompatible glibc, X11 oder XWayland und ALSA. Auf Debian 13 stehen die Laufzeitbibliotheken mit folgendem Befehl zur Verfügung.

```bash
docker build --output type=local,dest=dist/linux .
sudo apt install libasound2t64 libfontconfig1 libx11-6 libxcursor1 libxext6 libxfixes3 libxft2 libxinerama1 libxrandr2 libxrender1
chmod +x dist/linux/screacher-radio
./dist/linux/screacher-radio
```
