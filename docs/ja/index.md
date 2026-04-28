# LuaSkills 日本語ドキュメント

[English](../index.md) | [简体中文](../zh-CN/index.md) | [日本語](index.md) | [한국어](../ko/index.md) | [Español](../es/index.md) | [Français](../fr/index.md) | [Deutsch](../de/index.md) | [Português (BR)](../pt-BR/index.md)

[日本語 README](../../README.ja.md) | [英語ドキュメント](../index.md) | [中国語の詳細技術ドキュメント](../zh-CN/index.md)

ここは LuaSkills の日本語ドキュメント入口です。
この言語では製品概要とナビゲーションを提供します。Skill 作者向け手冊は英語で利用でき、より深い host / FFI 参照は引き続き中国語ドキュメントにあります。

## 推奨ルート

| 読者 | 入口 |
| --- | --- |
| 初めて読む人 | [日本語 README](../../README.ja.md) |
| 製品価値を知りたい人 | [なぜ LuaSkills なのか](product/why-luaskills.md) |
| Skill 作者 | [Skill development manual](../skill-development.md) |
| FFI / SDK 統合担当 | [FFI and SDK overview](../ffi/overview.md) |
| Database provider 実装者 | [Database provider overview](../providers/database-providers.md) |
| Runtime 境界を理解したい人 | [Runtime architecture overview](../architecture/runtime-model.md) |
| 詳細技術仕様が必要な人 | [中国語ドキュメント入口](../zh-CN/index.md) |

## Skill 命名規則

`skill_id` と各 `entry.name` は `^[a-z]([a-z0-9-]*[a-z0-9])?$` に一致する必要があります。
物理的な skill ディレクトリ名だけが `skill_id` の唯一のソースであり、`skill.yaml` で `skill_id` フィールドを宣言してはいけません。
canonical entry は `{skill_id}-{entry_name}` として公開され、競合時には安定した `-N` サフィックスが追加されることがあります。
GitHub 管理 skill では、リポジトリから派生した、または明示された `skill_id`、release zip のプレフィックス、checksum のプレフィックス、zip の最上位ディレクトリ、インストール先ディレクトリがすべて同一である必要があります。
asset は `{skill_id}-v{version}-skill.zip` と `{skill_id}-v{version}-checksums.txt` を使い、zip には `{skill_id}/skill.yaml` が含まれている必要があります。

## エコシステム

- [vulcan-codekit](https://github.com/LuaSkills/vulcan-codekit): 実運用に近い重要な LuaSkills 例。
- [demo-skill](https://github.com/LuaSkills/demo-skill): 最小 skill template。
- [luaskills-sdk-typescript](https://github.com/LuaSkills/luaskills-sdk-typescript): TypeScript / Node.js SDK。
- [luaskills-sdk-python](https://github.com/LuaSkills/luaskills-sdk-python): Python SDK。
- [luaskills-sdk-go](https://github.com/LuaSkills/luaskills-sdk-go): Go SDK。

## ローカル例

- [C FFI Demo](../../examples/ffi/c/README.md)
- [TypeScript FFI Demo](../../examples/ffi/typescript/README.md)
- [Standard Runtime Fixture](../../examples/ffi/standard_runtime/README.md)
- [Host Provider Demo](../../examples/ffi/host_provider_demo/README.md)
