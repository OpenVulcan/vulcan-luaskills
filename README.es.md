# LuaSkills

[English](README.md) | [简体中文](README.zh-CN.md) | [日本語](README.ja.md) | [한국어](README.ko.md) | [Español](README.es.md) | [Français](README.fr.md) | [Deutsch](README.de.md) | [Português (BR)](README.pt-BR.md)

[Documentación](docs/es/index.md) | [Plantilla de skill](https://github.com/LuaSkills/demo-skill) | [Ejemplo CodeKit](https://github.com/LuaSkills/vulcan-codekit)

LuaSkills es un runtime escrito en Rust para cargar, ejecutar y administrar skills basadas en Lua.
Permite que una aplicación host agregue herramientas scriptables, help estructurado, APIs de runtime, rutas de dependencias e integración con SQLite / LanceDB sin construir su propio runtime de plugins desde cero.

En una frase: LuaSkills ejecuta skills; el host decide cómo convertirlas en funciones de producto.

## Qué Es

LuaSkills es la capa de runtime central del ecosistema LuaSkills.
Está diseñado para aplicaciones que necesitan un sistema de skills controlado, no scripts sueltos.

Proporciona:

- Descubrimiento, carga, enumeración e invocación de skills.
- Strict help trees que el host puede mostrar como documentación, comandos, tools o UI.
- Inyección de APIs estándar `vulcan.*` y `vulcan.runtime.*`.
- Contexto de runtime para request actual, directorios de skill, recursos, dependencias y metadatos del cliente.
- Integración opcional con SQLite y LanceDB para skills con estado o memoria.
- API Rust, C ABI estándar y FFI pública `_json`.
- Rutas de integración SDK para TypeScript, Python y Go.

## Qué No Es

LuaSkills no controla toda la superficie del producto.

No es:

- Un MCP server por sí mismo.
- Un lector de configuración del host.
- Un calculador de budget del cliente.
- Un renderer de UI de producto.
- Una frontera sandbox para código Lua no confiable.

El host sigue controlando permisos, autenticación, UI, budgets, almacenamiento y exposición al usuario.

## Ecosistema

- [vulcan-codekit](https://github.com/LuaSkills/vulcan-codekit): ejemplo importante y cercano a producción que muestra navegación de código, inspección AST, búsqueda estructural, navegación Markdown y workflows de patch.
- [demo-skill](https://github.com/LuaSkills/demo-skill): plantilla mínima para aprender `skill.yaml`, runtime entries, help y layout de repositorio.
- [luaskills-sdk-typescript](https://github.com/LuaSkills/luaskills-sdk-typescript): SDK para TypeScript / Node.js.
- [luaskills-sdk-python](https://github.com/LuaSkills/luaskills-sdk-python): SDK para Python.
- [luaskills-sdk-go](https://github.com/LuaSkills/luaskills-sdk-go): SDK para Go.

## Documentación

- [Entrada en español](docs/es/index.md)
- [Por Qué LuaSkills](docs/es/product/why-luaskills.md)
- [Entrada en inglés](docs/index.md)
- [Documentación técnica detallada en chino](docs/zh-CN/index.md)

## Rutas De Integración

| Tipo de host | Ruta recomendada |
| --- | --- |
| Rust | Usar directamente el crate Rust. |
| C / C++ / host de bajo nivel | Usar el C ABI estándar. |
| TypeScript / Node.js | Preferir `luaskills-sdk-typescript`. |
| Python | Preferir `luaskills-sdk-python`. |
| Go | Elegir `luaskills-sdk-go` o C ABI estándar según callbacks y despliegue. |

## Inicio Rápido

Host Rust:

```toml
[dependencies]
luaskills = "0.2"
```

Comandos de desarrollo:

```bash
cargo check
cargo test --lib
```

Para aprender la forma de un skill:

1. [demo-skill](https://github.com/LuaSkills/demo-skill)
2. [vulcan-codekit](https://github.com/LuaSkills/vulcan-codekit)
3. [Skill development overview](docs/skill-development.md)

## Modelo De Confianza

Actualmente LuaSkills trata los skills como código confiable por defecto.
No ofrece una promesa de sandbox para paquetes Lua arbitrarios y no confiables.

El host debe decidir roots, skills instalables, acciones de gestión, modo de database provider y authority de cada operación.

## License

MIT
