# Screacher Radio

Kleiner Webradio-Player in Rust mit FLTK für Windows und Linux.

## Bedienung

- Unter **Hinzufügen…** einen Namen und eine HTTP(S)-URL angeben. Direkte MP3-Streams sowie M3U- und PLS-Playlists werden unterstützt. Eine Playlist erscheint als ein Sender und spielt ihren ersten Stream.
- **Play** startet den ausgewählten Sender. Beim Programmstart ist der zuletzt erfolgreich gespielte Sender vorausgewählt. **Stop** beendet die Wiedergabe.
- Die horizontalen Balken **L** und **R** zeigen die Spitzenpegel nach der Lautstärkeregelung. Bei Mono zeigen beide denselben Pegel.
- Sender lassen sich löschen, als Favoriten markieren und nach Name oder URL suchen. **Nur Favoriten** lässt sich mit der Suche kombinieren.
- Der **Sleeptimer** stoppt die Wiedergabe nach der eingegebenen Anzahl Minuten. Der Countdown läuft auch im Tray weiter.
- Unter **Einstellungen…** lässt sich das Minimieren in den Tray aktivieren. Das Tray-Icon öffnet den Player wieder und bietet ein Menü zum Beenden. Unter Linux braucht der Desktop dafür eine StatusNotifierItem-kompatible Tray-Leiste.

Senderliste, Favoriten, letzter Sender und Lautstärke speichert der Player in `settings.json`. Unter Windows liegt die Datei in `%APPDATA%\Screacher Radio\config`, unter Linux in `${XDG_CONFIG_HOME:-$HOME/.config}/screacherradio`.

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
