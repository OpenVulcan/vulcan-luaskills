# Documentação LuaSkills Em Português Do Brasil

[English](../index.md) | [简体中文](../zh-CN/index.md) | [日本語](../ja/index.md) | [한국어](../ko/index.md) | [Español](../es/index.md) | [Français](../fr/index.md) | [Deutsch](../de/index.md) | [Português (BR)](index.md)

[README em português do Brasil](../../README.pt-BR.md) | [Documentação em inglês](../index.md) | [Documentação técnica detalhada em chinês](../zh-CN/index.md)

Esta é a entrada em português do Brasil para a documentação do LuaSkills.
Esta camada oferece visão de produto e navegação; o manual para autores de skills está disponível em inglês, enquanto as referências profundas de host e FFI continuam em chinês.

## Caminho Recomendado

| Leitor | Início |
| --- | --- |
| Primeira visita | [README em português do Brasil](../../README.pt-BR.md) |
| Valor de produto | [Por Que LuaSkills](product/why-luaskills.md) |
| Autor de skills | [Skill development manual](../skill-development.md) |
| Integrador FFI / SDK | [FFI and SDK overview](../ffi/overview.md) |
| Implementador de database provider | [Database provider overview](../providers/database-providers.md) |
| Arquitetura runtime | [Runtime architecture overview](../architecture/runtime-model.md) |
| Especificação detalhada | [Documentação chinesa](../zh-CN/index.md) |

## Regras De Nomenclatura De Skills

`skill_id` e cada `entry.name` devem cumprir `^[a-z]([a-z0-9-]*[a-z0-9])?$`.
O nome físico do diretório do skill é a única fonte de `skill_id`; `skill.yaml` não deve declarar um campo `skill_id`.
As entradas canonical são expostas como `{skill_id}-{entry_name}` e podem receber um sufixo estável `-N` em caso de conflito.
Para skills gerenciados pelo GitHub, o `skill_id` derivado do repositório ou explícito, o prefixo do zip de release, o prefixo de checksums, o diretório superior do zip e o diretório instalado devem ser idênticos.
Os assets usam `{skill_id}-v{version}-skill.zip` e `{skill_id}-v{version}-checksums.txt`; o zip deve conter `{skill_id}/skill.yaml`.

## Ecossistema

- [vulcan-codekit](https://github.com/LuaSkills/vulcan-codekit): exemplo importante e próximo de produção.
- [demo-skill](https://github.com/LuaSkills/demo-skill): template mínimo de skill.
- [luaskills-sdk-typescript](https://github.com/LuaSkills/luaskills-sdk-typescript): SDK TypeScript / Node.js.
- [luaskills-sdk-python](https://github.com/LuaSkills/luaskills-sdk-python): SDK Python.
- [luaskills-sdk-go](https://github.com/LuaSkills/luaskills-sdk-go): SDK Go.

## Exemplos Locais

- [C FFI Demo](../../examples/ffi/c/README.md)
- [TypeScript FFI Demo](../../examples/ffi/typescript/README.md)
- [Standard Runtime Fixture](../../examples/ffi/standard_runtime/README.md)
- [Host Provider Demo](../../examples/ffi/host_provider_demo/README.md)
