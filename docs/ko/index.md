# LuaSkills 한국어 문서

[English](../index.md) | [简体中文](../zh-CN/index.md) | [日本語](../ja/index.md) | [한국어](index.md) | [Español](../es/index.md) | [Français](../fr/index.md) | [Deutsch](../de/index.md) | [Português (BR)](../pt-BR/index.md)

[한국어 README](../../README.ko.md) | [영어 문서](../index.md) | [중국어 상세 기술 문서](../zh-CN/index.md)

이 페이지는 LuaSkills의 한국어 문서 입구입니다.
이 언어에서는 제품 개요와 탐색 경로를 제공하며, 상세 API 참조는 현재 중국어 문서와 영어 overview로 연결합니다.

## 추천 경로

| 독자 | 시작점 |
| --- | --- |
| 처음 보는 사용자 | [한국어 README](../../README.ko.md) |
| 제품 가치 이해 | [왜 LuaSkills인가](product/why-luaskills.md) |
| Skill 작성자 | [Skill development overview](../skill-development.md) |
| FFI / SDK 통합 담당자 | [FFI and SDK overview](../ffi/overview.md) |
| Database provider 구현자 | [Database provider overview](../providers/database-providers.md) |
| Runtime 경계 이해 | [Runtime architecture overview](../architecture/runtime-model.md) |
| 상세 기술 사양 | [중국어 문서 입구](../zh-CN/index.md) |

## Skill 명명 규칙

`skill_id`와 각 `entry.name`은 `^[a-z]([a-z0-9-]*[a-z0-9])?$`와 일치해야 합니다.
물리적인 skill 디렉터리 이름만 `skill_id`의 유일한 출처이며, `skill.yaml`은 `skill_id` 필드를 선언하면 안 됩니다.
canonical entry는 `{skill_id}-{entry_name}` 형식으로 노출되며, 충돌이 있으면 안정적인 `-N` 접미사가 붙을 수 있습니다.
GitHub 관리 skill에서는 저장소에서 파생되었거나 명시된 `skill_id`, release zip 접두사, checksum 접두사, zip 최상위 디렉터리, 최종 설치 디렉터리가 모두 같아야 합니다.
asset은 `{skill_id}-v{version}-skill.zip`과 `{skill_id}-v{version}-checksums.txt`를 사용하며, zip에는 `{skill_id}/skill.yaml`이 포함되어야 합니다.

## 생태계

- [vulcan-codekit](https://github.com/LuaSkills/vulcan-codekit): 실제 제품 수준의 중요한 LuaSkills 예제.
- [demo-skill](https://github.com/LuaSkills/demo-skill): 최소 skill template.
- [luaskills-sdk-typescript](https://github.com/LuaSkills/luaskills-sdk-typescript): TypeScript / Node.js SDK.
- [luaskills-sdk-python](https://github.com/LuaSkills/luaskills-sdk-python): Python SDK.
- [luaskills-sdk-go](https://github.com/LuaSkills/luaskills-sdk-go): Go SDK.

## 로컬 예제

- [C FFI Demo](../../examples/ffi/c/README.md)
- [TypeScript FFI Demo](../../examples/ffi/typescript/README.md)
- [Standard Runtime Fixture](../../examples/ffi/standard_runtime/README.md)
- [Host Provider Demo](../../examples/ffi/host_provider_demo/README.md)
