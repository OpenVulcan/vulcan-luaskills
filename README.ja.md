# LuaSkills

[English](README.md) | [简体中文](README.zh-CN.md) | [日本語](README.ja.md) | [한국어](README.ko.md) | [Español](README.es.md) | [Français](README.fr.md) | [Deutsch](README.de.md) | [Português (BR)](README.pt-BR.md)

[ドキュメント](docs/ja/index.md) | [Skill テンプレート](https://github.com/LuaSkills/demo-skill) | [CodeKit サンプル](https://github.com/LuaSkills/vulcan-codekit)

LuaSkills は、Lua で書かれた skill を読み込み、実行し、管理するための Rust 製ランタイムです。
ホストアプリケーションは、スクリプト可能なツール、構造化された help、実行時 API、依存パス、SQLite / LanceDB 連携を、独自の plugin ランタイムを毎回作らずに追加できます。

一言で言えば、LuaSkills は skill を実行し、ホストはそれをどのように製品機能として公開するかを決めます。

## 概要

LuaSkills は LuaSkills エコシステムの中核ランタイムです。
単発スクリプトではなく、制御可能な skill システムを必要とするアプリケーション向けに設計されています。

主な機能:

- skill の検出、読み込み、entry 列挙、呼び出し。
- ホストがドキュメント、コマンドパレット、tool、UI に変換できる strict help tree。
- `vulcan.*` と `vulcan.runtime.*` の標準 Lua API 注入。
- リクエスト、skill ディレクトリ、リソース、依存ルート、クライアント情報の実行時コンテキスト注入。
- 状態管理や memory 系 skill のための SQLite / LanceDB 連携。
- Rust API、標準 C ABI、公共 `_json` FFI。
- TypeScript、Python、Go 向け SDK 連携パス。

## 対象ではないもの

LuaSkills は製品全体を所有しません。

対象ではありません:

- MCP server 本体。
- ホスト設定ファイルの読み取り器。
- クライアント budget 計算器。
- 製品 UI レンダラー。
- 任意の未信頼 Lua コード向け sandbox 境界。

権限、認証、UI、budget、保存場所、ユーザーにどう見せるかはホストが制御します。

## エコシステム

- [vulcan-codekit](https://github.com/LuaSkills/vulcan-codekit): 実運用に近い重要な LuaSkills サンプル。コードナビゲーション、AST 検査、構造検索、Markdown ナビゲーション、patch workflow を示します。
- [demo-skill](https://github.com/LuaSkills/demo-skill): `skill.yaml`、runtime entry、help、ディレクトリ構造を学ぶための最小 skill テンプレート。
- [luaskills-sdk-typescript](https://github.com/LuaSkills/luaskills-sdk-typescript): TypeScript / Node.js SDK。
- [luaskills-sdk-python](https://github.com/LuaSkills/luaskills-sdk-python): Python SDK。
- [luaskills-sdk-go](https://github.com/LuaSkills/luaskills-sdk-go): Go SDK。

## ドキュメント

- [日本語ドキュメント入口](docs/ja/index.md)
- [なぜ LuaSkills なのか](docs/ja/product/why-luaskills.md)
- [英語ドキュメント入口](docs/index.md)
- [中国語の詳細技術ドキュメント](docs/zh-CN/index.md)

## 統合パス

| ホスト種別 | 推奨パス |
| --- | --- |
| Rust | Rust crate を直接利用します。 |
| C / C++ / 低レベルホスト | 標準 C ABI を利用します。 |
| TypeScript / Node.js | `luaskills-sdk-typescript` を優先します。 |
| Python | `luaskills-sdk-python` を優先します。 |
| Go | callback と配布要件に応じて `luaskills-sdk-go` または標準 C ABI を選びます。 |

## クイックスタート

Rust ホスト:

```toml
[dependencies]
luaskills = "0.2"
```

開発用コマンド:

```bash
cargo check
cargo test --lib
```

skill の形を学ぶ場合:

1. [demo-skill](https://github.com/LuaSkills/demo-skill)
2. [vulcan-codekit](https://github.com/LuaSkills/vulcan-codekit)
3. [Skill development manual](docs/skill-development.md)

## Skill 命名規則

`skill_id` と各 `entry.name` は `^[a-z]([a-z0-9-]*[a-z0-9])?$` に一致する必要があります。
物理的な skill ディレクトリ名だけが `skill_id` の唯一のソースであり、`skill.yaml` で `skill_id` フィールドを宣言してはいけません。
canonical entry は `{skill_id}-{entry_name}` を使い、競合時には安定した `-N` サフィックスが追加されることがあります。
GitHub 管理 skill では、リポジトリから派生した、または明示された `skill_id`、release zip のプレフィックス、checksum のプレフィックス、zip の最上位ディレクトリ、インストール先ディレクトリがすべて同一である必要があります。
asset は `{skill_id}-v{version}-skill.zip` と `{skill_id}-v{version}-checksums.txt` を使い、zip には `{skill_id}/skill.yaml` が含まれている必要があります。

## 信頼モデル

現在の LuaSkills は skill を信頼済みコードとして扱います。
任意の未信頼 Lua package に対する sandbox セキュリティは提供しません。

ホストは root、skill のインストール可否、管理操作、database provider mode、操作 authority を明示的に決める必要があります。

## License

MIT
