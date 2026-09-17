<div align="center">

# LLM Usage Bar

> **2026-09-17:** The SwiftUI migration is discontinued. The shared macOS / Windows frontend uses React + TypeScript with Tauri 2 + Rust. See [current build instructions](README.md#cross-platform-architecture).

## Was deine KI-Coding-Tools wirklich kosten — Abo-Kontingent und API-Ausgaben, gebündelt in der Menüleiste

[![Platform](https://img.shields.io/badge/platform-macOS%2012%2B%20%7C%20Windows%2010%2B-lightgrey.svg)](#installation)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-orange.svg)](https://tauri.app/)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

[English](README.md) | [中文](README_ZH.md) | [日本語](README_JA.md) | Deutsch | [Changelog](CHANGELOG.md)

*Ein Fork von [CC Switch](https://github.com/farion1231/cc-switch), neu aufgebaut rund um Nutzungs-Tracking — siehe [Credits](#credits).*

</div>

## Was es tut

Wer mit Claude Code und Codex arbeitet, findet seinen Verbrauch über Stellen verteilt, die sich nie zu einer Summe fügen: ein Fünf-Stunden- und ein Wochenfenster im Claude-Abo, ein OAuth-Kontingent bei Codex und daneben ein Stapel API-Keys, die pro Token abgerechnet werden. Jedes hat seine eigene Seite, seine eigene Reset-Uhr — und keine gemeinsame Summe.

LLM Usage Bar liest all das lokal aus und stellt eine Antwort in die Menüleiste: **wie viel übrig ist und wie schnell du es verbrauchst.**

Es leitet deine Anfragen nicht um, verwaltet deine CLI-Konfiguration nicht und braucht kein Konto. Es liest die Sitzungsprotokolle und Kontingentdateien, die deine Tools ohnehin auf die Festplatte schreiben, und ruft Abrechnungs-Endpunkte mit Keys auf, die du selbst hinterlegst.

## Zwei Dinge werden erfasst

**Abo-Kontingent** — die prozentualen Fenster der Pläne, für die du bereits zahlst.

- **Claude** — beide Fenster, das Fünf-Stunden- und das Wochenfenster, gelesen aus der lokalen Planhistorie von Claude Desktop und der Statuszeilen-Brücke von Claude Code. Der Reset-Zeitpunkt wird gehalten und überlebt damit, welche Quelle auch immer zuletzt aktualisiert — gebunden an das Konto, das ihn geliefert hat.
- **Codex** — Kontingent über deine bestehende OAuth-Sitzung.
- **Coding-Pläne** — Kimi For Coding, Zhipu GLM (privat und Team), MiniMax und Volcano Ark. Pläne mit Nutzungslimit-Resets zeigen, wie viele übrig sind und wann sie verfallen.

**API-Ausgaben** — echtes Geld, pro Key.

- Gib einem Provider eine Liste **benannter API-Keys**; jeder meldet seine eigenen Tages- und Monatsausgaben, sein verbleibendes Budget und wann die Zahlen zuletzt geholt wurden. Provider mit mehr als einem Key zeigen zusätzlich eine Gesamtsumme.
- Die Ausgaben stammen vom Abrechnungs-Endpunkt des Anbieters selbst. Angebunden ist derzeit OpenRouter; andere Presets brauchen erst ihren eigenen Endpunkt, bevor sie etwas melden.
- Lege ein Tagesbudget pro Provider oder ein Gesamtbudget für APIs fest — die Leiste sagt dir, wenn du darüber hinausläufst.

## Die Verbrauchsampel

In der Menüleiste steht ein Indikator, keine Zahl, die du selbst deuten musst. Sie projiziert dein aktuelles Verbrauchstempo auf die verbleibende Zeit bis zum Reset:

- **Gesund** — in diesem Tempo endest du mit Spielraum
- **Warnung** — in diesem Tempo bist du vor dem Reset leer
- **Kritisch** — diesen Punkt hast du bereits überschritten

Das Popover zeigt die Grundlage der Einschätzung: das gemessene Tempo, die Projektion und die Zeit bis zum Fensterwechsel.

## Woher die Zahlen kommen

| Quelle                             | Was sie liefert                        | Wie                                                               |
| ---------------------------------- | -------------------------------------- | ----------------------------------------------------------------- |
| Claude Code / Codex Sitzungslogs   | Tokens, Modelle, Kosten pro Anfrage    | Import aus den JSONL-Dateien, die die CLIs lokal schreiben        |
| Claude Desktop Planhistorie        | Fünf-Stunden- und Wochenprozente       | Lokales JSON, von der App selbst aktualisiert                     |
| Claude Code Statuszeile            | Prozente **und** der Reset-Zeitpunkt   | Lokaler Cache, geschrieben während eine Sitzung die Zeile rendert |
| Codex OAuth                        | Abo-Kontingent                         | Anfrage, signiert mit deiner bestehenden OAuth-Sitzung            |
| Coding-Plan-Endpunkte              | Plankontingent und verbleibende Resets | Kimi, GLM, MiniMax per API-Key; Volcano Ark per AK/SK-Signatur    |
| Abrechnungs-Endpunkte der Anbieter | Ausgaben und Limits pro Key            | Direkter Aufruf mit dem hinterlegten Key                          |

Es gibt keinen lokalen Proxy und kein Abfangen von Anfragen. Was ein Tool nicht auf die Festplatte schreibt und kein Endpunkt meldet, weiß auch die App nicht.

## Aufschlüsselungen

Drei Tabs über denselben Zeitraum — heute, 7 Tage, 30 Tage oder ein Jahr:

- **Providers** — Ausgaben und Tokens je Provider, mit einer Aktivitäts-Heatmap über 12 Monate und einem stündlichen oder täglichen Trendverlauf
- **Models** — wohin das Geld tatsächlich geflossen ist
- **Agents** — welches Tool es ausgegeben hat, mit expliziten Zuordnungen von Agent zu Provider für Traffic, der sonst unzugeordnet bliebe

Jede Anfrage lässt sich einzeln ansehen, und die Kosten werden aus Preisen neu berechnet, die du kontrollierst: die offizielle Preisliste aktualisieren oder den Satz eines Modells pro Provider überschreiben.

## Installation

Es gibt noch keine veröffentlichten Releases. Baue es selbst — macOS 12 oder neuer:

```bash
pnpm install && pnpm build:local:mac
```

Das baut und signiert die App unter `release/tauri-target/release/bundle/macos/LLM Usage Bar.app` und hört dann auf. Ohne `--build-only` — also mit `./scripts/build_and_run.sh` — wird sie zusätzlich nach `/Applications` installiert und gestartet, mit einer Prüfung, die die vorherige App wiederherstellt, falls das neue Bundle nicht startet.

> **Das Upgrade ist einseitig.** Die App migriert ihre Datenbank beim ersten Start nach vorn und legt vorher automatisch ein Backup an. Nach der Migration kann ein älterer Build sie nicht mehr öffnen — die Versionsobergrenze verweigert den Zugriff, statt die Daten zu riskieren. Installiere bewusst.

## Deine Daten bleiben lokal

| Pfad                                | Inhalt                                                                |
| ----------------------------------- | --------------------------------------------------------------------- |
| `~/.llm-usage-bar/llm-usage-bar.db` | SQLite — Verbrauchsereignisse, Provider, Preise, Kontingent-Snapshots |
| `~/.llm-usage-bar/settings.json`    | UI-Einstellungen auf Geräteebene                                      |
| `~/.llm-usage-bar/backups/`         | Automatische Backups vor Migrationen, standardmäßig die letzten 10    |
| `~/.llm-usage-bar/logs/`            | Anwendungsprotokoll                                                   |

API-Keys liegen im macOS-Schlüsselbund oder im Windows Credential Manager — nie in der Datenbank und nie in den Logs. Ausgabenbeträge werden nie in die Logdatei geschrieben.

Synchronisierung ist optional: Die Datenbank kann in einem eigenen Konfigurationsverzeichnis liegen (iCloud, Dropbox, OneDrive, NAS) oder auf WebDAV bzw. S3-kompatiblen Speicher geschoben werden. Standardmäßig aus.

## FAQ

<details>
<summary><strong>Muss ich ändern, wie ich Claude Code oder Codex benutze?</strong></summary>

Nein. Die App liest Dateien, die diese Tools ohnehin schreiben. Nichts wird umgeleitet, eingeschleust oder umgeschrieben. Deinstallierst du sie, bleiben deine CLIs unberührt.

</details>

<details>
<summary><strong>Warum zeigt ein Provider keine Ausgaben?</strong></summary>

Weil Ausgaben vom Abrechnungs-Endpunkt des Anbieters kommen und nur Presets sie melden können, die an einen solchen angebunden sind. Angebunden ist OpenRouter (`GET /api/v1/key`, begrenzt auf den Key, der den Aufruf authentifiziert). Für andere muss der jeweilige Endpunkt erst ergänzt werden. Ein Provider ohne Endpunkt und ohne Zuordnung aus Sitzungslogs zeigt zu Recht nichts an.

</details>

<details>
<summary><strong>Warum ist die Summe eines Keys als „ersetzte Zugangsdaten“ markiert?</strong></summary>

Weil sie das ist. Einen Key zu ersetzen ordnet die bereits angefallenen Ausgaben des alten Keys nicht rückwirkend neu zu. Deshalb werden diese Zahlen ausgewiesen, statt still in die aktuelle Summe einzufließen.

</details>

<details>
<summary><strong>Claude zeigt einen Prozentwert, aber keine Reset-Zeit. Warum?</strong></summary>

Von Claudes zwei lokalen Quellen führt nur eine einen Reset-Zeitpunkt mit — die Statuszeilen-Brücke von Claude Code, die nur schreibt, während eine Terminal-Sitzung ihre Statuszeile rendert. Die Planhistorie von Claude Desktop hat die Prozente, aber nie den Reset. Die App hält einen einmal gesehenen Reset fest und liefert ihn aus, bis er verstreicht; lief die Brücke nie, gibt es jedoch nichts festzuhalten. Dann sagt die App das, statt einen Platzhalter zu zeigen.

</details>

<details>
<summary><strong>Kann es ein Abo erfassen, das auf einem anderen Rechner genutzt wird?</strong></summary>

Nein. Alles wird aus den lokalen Dateien dieses Rechners und aus Key-bezogenen Endpunkten gelesen. Ein anderswo genutzter Key ist für einen Key-bezogenen Endpunkt unsichtbar, und die Sitzungslogs eines anderen Rechners liegen nicht hier vor.

</details>

<details>
<summary><strong>Welche Sprachen unterstützt die Oberfläche?</strong></summary>

English, 简体中文, 繁體中文 und 日本語.

</details>

## Dokumentation

- [Changelog](CHANGELOG.md)
- [Mitwirken](CONTRIBUTING.md) · [Sicherheitsrichtlinie](SECURITY.md) · [Support](SUPPORT.md)

> `docs/user-manual/` beschreibt weiterhin die entfernten Funktionen zum Provider-Wechsel sowie Proxy, MCP, Prompts und Skills — bis zur Überarbeitung wird von hier nicht darauf verlinkt.

## Gebaut mit

[Tauri 2](https://tauri.app/) · Rust · React 19 · TypeScript · SQLite

## Credits

LLM Usage Bar begann als Fork von [CC Switch](https://github.com/farion1231/cc-switch) von Jason Young und baut weiterhin auf dessen Provider-, Speicher-, Sync- und Metering-Fundament auf. Die Funktionen von CC Switch für Provider-Wechsel, Proxy, MCP, Prompts und Skills wurden entfernt; die Schichten für Quota, Ingestion, Aggregation, Dashboard, Routing und Native Bridge sind neu in diesem Projekt.

## Lizenz

[MIT](LICENSE)
