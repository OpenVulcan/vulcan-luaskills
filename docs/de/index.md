# LuaSkills Dokumentation Auf Deutsch

[English](../index.md) | [简体中文](../zh-CN/index.md) | [日本語](../ja/index.md) | [한국어](../ko/index.md) | [Español](../es/index.md) | [Français](../fr/index.md) | [Deutsch](index.md) | [Português (BR)](../pt-BR/index.md)

[Deutsches README](../../README.de.md) | [Englische Dokumentation](../index.md) | [Ausführliche chinesische technische Dokumentation](../zh-CN/index.md)

Dies ist der deutsche Einstieg in die LuaSkills-Dokumentation.
Diese Ebene bietet Produktüberblick und Navigation; die vollständige technische Referenz wird derzeit auf Chinesisch gepflegt, mit englischen Overviews.

## Empfohlener Pfad

| Leser | Einstieg |
| --- | --- |
| Erstbesucher | [Deutsches README](../../README.de.md) |
| Produktwert | [Warum LuaSkills](product/why-luaskills.md) |
| Skill-Autor | [Skill development overview](../skill-development.md) |
| FFI / SDK Integrator | [FFI and SDK overview](../ffi/overview.md) |
| Database-provider Implementierung | [Database provider overview](../providers/database-providers.md) |
| Runtime-Architektur | [Runtime architecture overview](../architecture/runtime-model.md) |
| Detaillierte Spezifikation | [Chinesische Dokumentation](../zh-CN/index.md) |

## Skill-Namensregeln

`skill_id` und jedes `entry.name` müssen `^[a-z]([a-z0-9-]*[a-z0-9])?$` erfüllen.
Der physische Skill-Ordnername ist die einzige Quelle für `skill_id`; `skill.yaml` darf kein `skill_id`-Feld deklarieren.
Canonical Entries werden als `{skill_id}-{entry_name}` veröffentlicht und können bei Konflikten ein stabiles `-N`-Suffix erhalten.
Für GitHub-verwaltete Skills müssen der aus dem Repository abgeleitete oder explizite `skill_id`, Release-Zip-Präfix, Checksum-Präfix, oberster Zip-Ordner und Installationsordner identisch sein.
Release-Dateien verwenden `{skill_id}-v{version}-skill.zip` und `{skill_id}-v{version}-checksums.txt`; das Zip muss `{skill_id}/skill.yaml` enthalten.

## Ökosystem

- [vulcan-codekit](https://github.com/LuaSkills/vulcan-codekit): wichtiges produktnahes Beispiel.
- [demo-skill](https://github.com/LuaSkills/demo-skill): minimales Skill-Template.
- [luaskills-sdk-typescript](https://github.com/LuaSkills/luaskills-sdk-typescript): TypeScript / Node.js SDK.
- [luaskills-sdk-python](https://github.com/LuaSkills/luaskills-sdk-python): Python SDK.
- [luaskills-sdk-go](https://github.com/LuaSkills/luaskills-sdk-go): Go SDK.

## Lokale Beispiele

- [C FFI Demo](../../examples/ffi/c/README.md)
- [TypeScript FFI Demo](../../examples/ffi/typescript/README.md)
- [Standard Runtime Fixture](../../examples/ffi/standard_runtime/README.md)
- [Host Provider Demo](../../examples/ffi/host_provider_demo/README.md)
