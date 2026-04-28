# LuaSkills

[English](README.md) | [简体中文](README.zh-CN.md) | [日本語](README.ja.md) | [한국어](README.ko.md) | [Español](README.es.md) | [Français](README.fr.md) | [Deutsch](README.de.md) | [Português (BR)](README.pt-BR.md)

[Dokumentation](docs/de/index.md) | [Skill-Template](https://github.com/LuaSkills/demo-skill) | [CodeKit-Beispiel](https://github.com/LuaSkills/vulcan-codekit)

LuaSkills ist eine Rust-basierte Runtime zum Laden, Ausführen und Verwalten von Lua-basierten Skills.
Host-Anwendungen können scriptbare Tools, strukturierte Hilfe, Runtime-APIs, Dependency-Pfade und SQLite / LanceDB-Integration hinzufügen, ohne jedes Mal eine eigene Plugin-Runtime zu bauen.

Kurz gesagt: LuaSkills führt Skills aus; der Host entscheidet, wie daraus Produktfunktionen werden.

## Was Es Ist

LuaSkills ist die zentrale Runtime-Schicht des LuaSkills-Ökosystems.
Es ist für Anwendungen gedacht, die ein kontrolliertes Skill-System statt einzelner Skripte benötigen.

Es bietet:

- Skill-Erkennung, Laden, Entry-Auflistung und Aufruf.
- Strict help trees, die der Host als Dokumentation, Command Palette, Tools oder UI rendern kann.
- Standardisierte Lua-APIs unter `vulcan.*` und `vulcan.runtime.*`.
- Runtime-Kontext für aktuelle Requests, Skill-Verzeichnisse, Ressourcen, Dependencies und Client-Metadaten.
- Optionale SQLite- und LanceDB-Bindings für zustandsbehaftete oder memory-orientierte Skills.
- Rust API, Standard C ABI und öffentliche `_json` FFI.
- SDK-Integrationspfade für TypeScript, Python und Go.

## Was Es Nicht Ist

LuaSkills besitzt nicht die gesamte Produktoberfläche.

Es ist kein:

- eigenständiger MCP server.
- Leser für Host-Konfigurationsdateien.
- Client-Budget-Rechner.
- Produkt-UI-Renderer.
- Sandbox-Grenze für nicht vertrauenswürdigen Lua-Code.

Berechtigungen, Authentifizierung, UI, Budgets, Speicherorte und Benutzeroberflächen bleiben Aufgabe des Hosts.

## Ökosystem

- [vulcan-codekit](https://github.com/LuaSkills/vulcan-codekit): wichtiges produktnahes LuaSkills-Beispiel mit Code-Navigation, AST-Inspektion, struktureller Suche, Markdown-Navigation und Patch-Workflows.
- [demo-skill](https://github.com/LuaSkills/demo-skill): minimales Template zum Lernen von `skill.yaml`, Runtime Entries, Help-Dateien und Repository-Layout.
- [luaskills-sdk-typescript](https://github.com/LuaSkills/luaskills-sdk-typescript): TypeScript / Node.js SDK.
- [luaskills-sdk-python](https://github.com/LuaSkills/luaskills-sdk-python): Python SDK.
- [luaskills-sdk-go](https://github.com/LuaSkills/luaskills-sdk-go): Go SDK.

## Dokumentation

- [Deutscher Dokumentationseinstieg](docs/de/index.md)
- [Warum LuaSkills](docs/de/product/why-luaskills.md)
- [Englischer Dokumentationseinstieg](docs/index.md)
- [Ausführliche chinesische technische Dokumentation](docs/zh-CN/index.md)

## Integrationspfade

| Host-Typ | Empfohlener Pfad |
| --- | --- |
| Rust | Rust crate direkt verwenden. |
| C / C++ / Low-Level-Host | Standard C ABI verwenden. |
| TypeScript / Node.js | `luaskills-sdk-typescript` bevorzugen. |
| Python | `luaskills-sdk-python` bevorzugen. |
| Go | Je nach Callback- und Deployment-Anforderungen `luaskills-sdk-go` oder Standard C ABI wählen. |

## Schnellstart

Rust-Host:

```toml
[dependencies]
luaskills = "0.2"
```

Entwicklungsbefehle:

```bash
cargo check
cargo test --lib
```

Um die Skill-Struktur zu lernen:

1. [demo-skill](https://github.com/LuaSkills/demo-skill)
2. [vulcan-codekit](https://github.com/LuaSkills/vulcan-codekit)
3. [Skill development overview](docs/skill-development.md)

## Vertrauensmodell

LuaSkills behandelt Skills derzeit standardmäßig als vertrauenswürdigen Code.
Es bietet keine Sandbox-Sicherheitszusage für beliebige nicht vertrauenswürdige Lua-Pakete.

Der Host muss roots, installierbare Skills, Verwaltungsaktionen, database provider mode und operation authority festlegen.

## License

MIT
