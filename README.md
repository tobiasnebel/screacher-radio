# Screacher Radio

Kleiner Webradio-Player in Rust mit FLTK für Windows und Linux.

Pro Benutzer läuft eine Instanz. Ein erneuter Start holt den laufenden Player aus dem Tray oder der Taskleiste hervor und fordert den Fokus für sein Fenster an. Nach einem Absturz wird die Instanzsperre automatisch freigegeben.

## Bedienung

- **Letzten Sender beim Programmstart abspielen** unter **Einstellungen** aktiviert Autoplay. Die Option ist anfangs ausgeschaltet und startet nur einen noch vorhandenen zuletzt gespielten Sender.
- Die Medientaste **Play/Pause** startet bzw. stoppt den Stream auch bei minimiertem Fenster oder im Tray. Unter Linux benötigt der Desktop MPRIS-Unterstützung und einen D-Bus-Sitzungsbus.
- Das Hauptfenster mit leicht gerundeten Ecken lässt sich mit gedrückter linker Maustaste auf einer freien Fläche, im Streaminfo-Panel, auf der Statuszeile und auf freien Stellen der Menüleiste verschieben. Oben rechts minimiert der Strich den Player gemäß der Tray-Einstellung. Das Kreuz und **Datei → Beenden** schließen den Player.
- Das Theme **Light** unter **Einstellungen → Theme** verwendet einen perlmuttweißen Hintergrund mit blauer Schrift.
- Unter **Hinzufügen…** einen Namen und eine HTTP(S)-URL angeben. Direkte MP3-Streams sowie M3U- und PLS-Playlists werden unterstützt. Eine Playlist erscheint als ein Sender und spielt ihren ersten Stream.
- Das Dreieck startet den ausgewählten Sender, das Quadrat stoppt die Wiedergabe. Beim Programmstart ist der zuletzt erfolgreich gespielte Sender vorausgewählt.
- Rechts neben der Senderliste stehen die Laufzeit seit Wiedergabestart und, wenn der Stream ICY-Metadaten liefert, Titel und Interpret. Ein Eintrag der Form `Interpret - Titel` wird in beide Felder aufgeteilt.
- Die Lautstärke lässt sich auch mit dem Mausrad über dem Regler ändern. Die horizontalen Balken **L** und **R** zeigen die Spitzenpegel mit einem Verlauf von Grün über Gelb zu Rot. Bei Mono zeigen beide denselben Pegel.
- Ein Klick auf das Lautsprechersymbol schaltet den Ton stumm oder wieder ein. Der Lautstärkewert bleibt erhalten und lässt sich auch bei stummgeschaltetem Ton ändern.
- Unter **Einstellungen → Audioausgabegerät** kannst du ein Gerät fest auswählen oder das Standardgerät verwenden. Die Liste aktualisiert sich, während die Einstellungen offen sind. Ist das gespeicherte Gerät nicht verfügbar, wird das Standardgerät gewählt. Beim Wechsel kann der Ton kurz aussetzen. Sender und Sleeptimer laufen weiter.
- Sender lassen sich löschen, über den Stern als Favoriten markieren und nach Name oder URL suchen. Ein voller Stern kennzeichnet einen Favoriten. **Nur Favoriten** lässt sich mit der Suche kombinieren.
- Das Uhrsymbol öffnet den **Sleeptimer**. Er stoppt die Wiedergabe nach 15, 30, 45 oder 60 Minuten oder nach einer frei eingegebenen Dauer. Einen laufenden Timer kannst du um 5, 10 oder 15 Minuten verlängern. Beim Schließen des Dialogs und im Tray läuft der Countdown weiter.
- Unter **Einstellungen…** findest du **Immer im Vordergrund**, die Themes **Standard** und **Dunkel** sowie das Minimieren in den Tray. Theme-Wechsel werden sofort sichtbar. **Esc** blendet bei aktiver Tray-Option und verfügbarem Tray das Hauptfenster aus. Ein Linksklick auf das Tray-Icon holt das Hauptfenster hervor. Ein Rechtsklick öffnet das Menü. Unter Linux braucht der Desktop eine StatusNotifierItem-kompatible Tray-Leiste und für **Immer im Vordergrund** einen Fenstermanager mit EWMH-Unterstützung.

Der Player liest und speichert Sender und Favoriten in `stations.yaml` direkt neben der Programmdatei. Beim ersten Start ohne diese Datei übernimmt er vorhandene Sender aus `settings.json`. Zum Ändern der Senderliste muss der Programmordner beschreibbar sein. Die YAML-Datei kannst du mit der Programmdatei in einen anderen Ordner mitnehmen.

```yaml
- name: Mein Sender
  url: https://example.org/radio.mp3
  favorite: true
```

Letzter Sender und Lautstärke bleiben in `settings.json`. Unter Windows liegt die Datei in `%APPDATA%\Screacher Radio\config`, unter Linux in `${XDG_CONFIG_HOME:-$HOME/.config}/screacherradio`.

## Downloads aus GitHub Actions

Der Workflow [Build binaries](.github/workflows/build.yml) läuft bei einem Push oder per **Actions → Build binaries → Run workflow**. Nach erfolgreichem Build stehen im jeweiligen Lauf unter **Artifacts** die Windows-EXE und das Linux-Archiv `tar.gz` für x86_64 zum Download bereit. Der Linux-Build nutzt das Debian-Image aus dem Dockerfile. Die Laufzeitabhängigkeiten stehen unten im Abschnitt Linux.

Wird auf GitHub ein Release veröffentlicht, baut derselbe Workflow den Stand des Release-Tags und hängt beide Dateien unter **Releases → Assets** an. Dort bleiben die Downloads über die Aufbewahrungsfrist der Actions-Artefakte hinaus verfügbar. Das nachträgliche Anhängen setzt voraus, dass das Repository Änderungen an veröffentlichten Release-Assets erlaubt.

Im About-Dialog entspricht die Version eines Release-Builds dem GitHub-Tag, etwa `v0.2.0`. Push- und manuell ausgelöste Builds tragen die Cargo-Version mit `-snapshot` und der abgekürzten Commit-ID, etwa `0.1.0-snapshot+abc1234`. Lokale Builds zeigen die Snapshot-Version ohne Commit-ID.

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

Das Linux-Archiv enthält einen Desktop-Starter und das Logo für den Anwendungsstarter. Die Installation für den eigenen Benutzer erfolgt aus dem entpackten Verzeichnis. Der Ordner `~/.local/bin` muss im `PATH` der Desktop-Sitzung enthalten sein.

```bash
install -Dm755 screacher-radio ~/.local/bin/screacher-radio
install -Dm644 screacher-radio.desktop ~/.local/share/applications/screacher-radio.desktop
install -Dm644 screacher-radio.png ~/.local/share/icons/hicolor/512x512/apps/screacher-radio.png
```

## Screenshots

![Main Window](./media/screenshots/main_window.png "Main Window")
