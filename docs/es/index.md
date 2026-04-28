# Documentación De LuaSkills En Español

[English](../index.md) | [简体中文](../zh-CN/index.md) | [日本語](../ja/index.md) | [한국어](../ko/index.md) | [Español](index.md) | [Français](../fr/index.md) | [Deutsch](../de/index.md) | [Português (BR)](../pt-BR/index.md)

[README en español](../../README.es.md) | [Documentación en inglés](../index.md) | [Documentación técnica detallada en chino](../zh-CN/index.md)

Esta es la entrada en español para la documentación de LuaSkills.
Esta capa ofrece una visión de producto y navegación; el manual para autores de skills está disponible en inglés, mientras que las referencias profundas de host y FFI se mantienen en chino.

## Ruta Recomendada

| Lector | Inicio |
| --- | --- |
| Primera visita | [README en español](../../README.es.md) |
| Valor de producto | [Por Qué LuaSkills](product/why-luaskills.md) |
| Autor de skills | [Skill development manual](../skill-development.md) |
| Integrador FFI / SDK | [FFI and SDK overview](../ffi/overview.md) |
| Implementador de database provider | [Database provider overview](../providers/database-providers.md) |
| Arquitectura runtime | [Runtime architecture overview](../architecture/runtime-model.md) |
| Especificación detallada | [Documentación china](../zh-CN/index.md) |

## Reglas De Nomenclatura De Skills

`skill_id` y cada `entry.name` deben cumplir `^[a-z]([a-z0-9-]*[a-z0-9])?$`.
El nombre físico del directorio del skill es la única fuente de `skill_id`; `skill.yaml` no debe declarar un campo `skill_id`.
Las entradas canonical se exponen como `{skill_id}-{entry_name}` y pueden recibir un sufijo estable `-N` si hay conflictos.
Para skills gestionados por GitHub, el `skill_id` derivado del repositorio o explícito, el prefijo del zip de release, el prefijo de checksums, el directorio superior del zip y el directorio instalado deben ser idénticos.
Los assets usan `{skill_id}-v{version}-skill.zip` y `{skill_id}-v{version}-checksums.txt`; el zip debe contener `{skill_id}/skill.yaml`.

## Ecosistema

- [vulcan-codekit](https://github.com/LuaSkills/vulcan-codekit): ejemplo importante y cercano a producción.
- [demo-skill](https://github.com/LuaSkills/demo-skill): template mínimo de skill.
- [luaskills-sdk-typescript](https://github.com/LuaSkills/luaskills-sdk-typescript): SDK TypeScript / Node.js.
- [luaskills-sdk-python](https://github.com/LuaSkills/luaskills-sdk-python): SDK Python.
- [luaskills-sdk-go](https://github.com/LuaSkills/luaskills-sdk-go): SDK Go.

## Ejemplos Locales

- [C FFI Demo](../../examples/ffi/c/README.md)
- [TypeScript FFI Demo](../../examples/ffi/typescript/README.md)
- [Standard Runtime Fixture](../../examples/ffi/standard_runtime/README.md)
- [Host Provider Demo](../../examples/ffi/host_provider_demo/README.md)
