# Documentation LuaSkills En Français

[English](../index.md) | [简体中文](../zh-CN/index.md) | [日本語](../ja/index.md) | [한국어](../ko/index.md) | [Español](../es/index.md) | [Français](index.md) | [Deutsch](../de/index.md) | [Português (BR)](../pt-BR/index.md)

[README français](../../README.fr.md) | [Documentation anglaise](../index.md) | [Documentation technique détaillée en chinois](../zh-CN/index.md)

Cette page est l'entrée française de la documentation LuaSkills.
Elle fournit la vue produit et la navigation; le manuel pour auteurs de skills est disponible en anglais, tandis que les références détaillées host et FFI restent maintenues en chinois.

## Parcours Recommandé

| Lecteur | Départ |
| --- | --- |
| Première visite | [README français](../../README.fr.md) |
| Valeur produit | [Pourquoi LuaSkills](product/why-luaskills.md) |
| Auteur de skills | [Skill development manual](../skill-development.md) |
| Intégrateur FFI / SDK | [FFI and SDK overview](../ffi/overview.md) |
| Implémentation database provider | [Database provider overview](../providers/database-providers.md) |
| Architecture runtime | [Runtime architecture overview](../architecture/runtime-model.md) |
| Spécification détaillée | [Documentation chinoise](../zh-CN/index.md) |

## Règles De Nommage Des Skills

`skill_id` et chaque `entry.name` doivent respecter `^[a-z]([a-z0-9-]*[a-z0-9])?$`.
Le nom physique du dossier du skill est la seule source de `skill_id`; `skill.yaml` ne doit pas déclarer de champ `skill_id`.
Les entrées canonical sont exposées sous la forme `{skill_id}-{entry_name}` et peuvent recevoir un suffixe stable `-N` en cas de conflit.
Pour les skills gérés par GitHub, le `skill_id` dérivé du dépôt ou explicite, le préfixe du zip de release, le préfixe des checksums, le dossier racine du zip et le dossier d'installation doivent être identiques.
Les assets utilisent `{skill_id}-v{version}-skill.zip` et `{skill_id}-v{version}-checksums.txt`; le zip doit contenir `{skill_id}/skill.yaml`.

## Écosystème

- [vulcan-codekit](https://github.com/LuaSkills/vulcan-codekit) : exemple important et proche de la production.
- [demo-skill](https://github.com/LuaSkills/demo-skill) : template minimal de skill.
- [luaskills-sdk-typescript](https://github.com/LuaSkills/luaskills-sdk-typescript) : SDK TypeScript / Node.js.
- [luaskills-sdk-python](https://github.com/LuaSkills/luaskills-sdk-python) : SDK Python.
- [luaskills-sdk-go](https://github.com/LuaSkills/luaskills-sdk-go) : SDK Go.

## Exemples Locaux

- [C FFI Demo](../../examples/ffi/c/README.md)
- [TypeScript FFI Demo](../../examples/ffi/typescript/README.md)
- [Standard Runtime Fixture](../../examples/ffi/standard_runtime/README.md)
- [Host Provider Demo](../../examples/ffi/host_provider_demo/README.md)
