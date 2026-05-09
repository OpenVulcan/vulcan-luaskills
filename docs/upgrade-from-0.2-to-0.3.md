# LuaSkills `0.2` to `0.3` Upgrade Guide

This guide is for hosts, SDK users, and release maintainers who already use `LuaSkills 0.2.x` and need a concise explanation of what changed in `0.3.x`, what code usually stays the same, and what integration or packaging assumptions must change.

## 1. Scope

This guide covers:

- `luaskills` crate / FFI / demos: `0.2.x -> 0.3.x`
- `luaskills-packages`: the separated `0.1.x` compatible series
- TypeScript / Python / Go SDK upgrades to `0.3.x`

The current stable combination is:

- `LuaSkills/luaskills`: `0.4.0`
- `LuaSkills/luaskills-packages`: `0.1.6`

## 2. Short Version

In practice:

1. **Most Rust hosts only need to bump the crate dependency to `0.3`.**
2. **If you manage runtime asset downloads, FFI install scripts, or demo dependency scripts yourself, you must adapt to the new `luaskills-packages` split.**
3. **If you still rely on legacy directory-style roots or old runtime-session compatibility behavior, move to the formal `0.3` interfaces now.**
4. **If you assemble packaged runtimes yourself, `0.3` now treats missing `luaskills-packages` metadata as a hard error.**

## 3. What Changed in `0.3`

### 3.1 `luaskills-packages` moved out of the main repository

In `0.2.x`, Lua runtime dependencies, LuaRocks package lists, native dependency packaging, and runtime assets were mostly produced together from the main repository.

In `0.3.x`, responsibilities are split:

| Component | Responsibility |
| --- | --- |
| `LuaSkills/luaskills-packages` | `lua-runtime-packages-*`, `lua-deps-*`, Lua package inventories, help metadata, license metadata |
| `LuaSkills/luaskills` | crate, core native library, `luaskills-ffi-sdk-*`, demo assets |
| SDKs | compose core assets with packages assets |

The important consequence is that **`lua-runtime-*` and `lua-deps-*` are no longer published by `LuaSkills/luaskills`**.

### 3.2 Runtime assets now come from two sources

The new runtime asset model is:

| Asset | Source repository |
| --- | --- |
| `luaskills-ffi-sdk-{platform}.tar.gz` | `LuaSkills/luaskills` |
| `lua-runtime-packages-{platform}.tar.gz` | `LuaSkills/luaskills-packages` |
| `lua-deps-{platform}.tar.gz` | `LuaSkills/luaskills-packages` |

If you used to assume that “downloading the main repository release gives a complete Lua runtime”, that assumption no longer holds in `0.3`.

### 3.3 Packaged runtimes now require `luaskills-packages` metadata

`0.3` adds strict packaged runtime validation. A packaged runtime is now expected to contain at least:

- `resources/lua-runtime-manifest.json`
- `resources/luaskills-packages-manifest.json`
- `resources/luaskills-packages/install-manifest.json`
- `resources/luaskills-packages/lua_packages.txt`
- `resources/luaskills-packages/platform-support.json`
- `resources/luaskills-packages/THIRD_PARTY_LICENSES.json`
- `resources/luaskills-packages/THIRD_PARTY_NOTICES.md`
- `resources/luaskills-packages/help/index.json`
- `resources/luaskills-packages/help/packages`
- `resources/luaskills-packages/help/modules`
- `licenses/luaskills-packages/index.json`

If these files are missing, `0.3` treats the runtime as incomplete and fails early instead of continuing with an invalid package layout.

### 3.4 Legacy compatibility paths were intentionally reduced

`0.3` deliberately converges on the latest formal protocol surface:

- authority-bound runtime-session calls no longer silently fall back to old public endpoints
- legacy directory-style roots are gone in favor of formal roots / root-chain APIs
- the main release pipeline no longer owns local Lua deps compilation and complete runtime publishing

## 4. Do Rust hosts need code changes?

### 4.1 Most direct Rust hosts only need a dependency bump

If your host already uses the formal Rust API:

- `LuaEngine`
- `RuntimeSkillRoot`
- `load_from_roots(...)`

then upgrading is usually just:

```toml
[dependencies]
luaskills = "0.3"
```

Most hosts in this category do **not** need a new call flow.

### 4.2 Cases that do require changes

#### Case A: you deserialize host capability JSON yourself

If you manage your own host capability JSON, explicitly include `enable_managed_io_compat` instead of relying on the old missing-field compatibility behavior.

#### Case B: you build `LuaRuntimeCapabilityOptions` with struct literals

If you do not use `Default::default()` and instead construct the struct explicitly, you now need to include `enable_managed_io_compat`.

#### Case C: you still use directory-style root wrappers

`0.3` converges on `RuntimeSkillRoot + load_from_roots / reload_from_roots`. Old directory-style wrappers should be replaced.

#### Case D: you assemble packaged runtimes yourself

If you do not consume the official runtime assets directly and instead package or trim runtime directories yourself, you must include the new `luaskills-packages` metadata tree. Otherwise packaged runtime initialization fails in `0.3`.

## 5. What FFI / SDK / install-script users need to know

### 5.1 SDKs no longer depend only on the main repository release

SDKs now use a two-source model:

- core assets follow the `luaskills` / SDK release version
- packages assets follow a compatible packages series

The current stable rule is:

- core: `0.3.x`
- packages: `0.1.x`

When no exact packages patch version is specified, SDK installers resolve the newest published patch from the compatible `0.1` series of `LuaSkills/luaskills-packages`.

### 5.2 Demo and runtime-fetch scripts changed meaningfully

`fetch_runtime_deps.ps1` / `fetch_runtime_deps.sh` now compose:

- `luaskills-ffi-sdk-*` from `LuaSkills/luaskills`
- `lua-runtime-packages-*` from `LuaSkills/luaskills-packages`

And `install_lua_deps.ps1` / `install_lua_deps.sh` now download prebuilt deps from `LuaSkills/luaskills-packages`.

If you copied old scripts into another repository or maintain your own wrapper scripts, update them.

## 6. Release Flow Changes

The recommended ecosystem release order is now:

1. Publish `LuaSkills/luaskills-packages`
2. Publish `LuaSkills/luaskills`
3. Publish the TypeScript SDK
4. Publish the Python SDK
5. Publish the Go SDK
6. Run each SDK repository’s examples release workflow

This ensures:

- packages assets already exist
- demos and SDK installers do not point at unpublished runtime assets
- examples release workflows consume final published assets

## 7. Upgrade Checklist

### Direct Rust hosts

- [ ] `Cargo.toml` uses `luaskills = "0.3"`
- [ ] the host still uses `RuntimeSkillRoot + load_from_roots(...)`
- [ ] custom host capability configuration explicitly handles `enable_managed_io_compat`

### FFI / SDK / demo users

- [ ] do not assume the main repository release includes a complete `lua-runtime-*`
- [ ] accept `lua-runtime-packages-*` and `lua-deps-*` from `luaskills-packages`
- [ ] if you use packaged runtimes, verify the `resources/luaskills-packages*` metadata files are present

### Release maintainers

- [ ] publish in `luaskills-packages -> luaskills -> SDKs -> examples` order
- [ ] SDK defaults use the `0.1` packages series
- [ ] old directory-style roots or runtime-session compatibility notes are removed from active docs and scripts

## 8. FAQ

### Does every Rust host need business-logic changes for `0.3`?

No. Most direct Rust integrations only need the dependency bump and, in some cases, small host configuration updates.

### Why is `lua-runtime-*` no longer in the main `luaskills` release?

Because `0.3` moved runtime packages and native dependency bundles into the dedicated `LuaSkills/luaskills-packages` repository.

### Why does packaged runtime loading fail immediately in `0.3` when files are missing?

Because `0.3` now validates the `luaskills-packages` metadata tree up front, instead of allowing incomplete runtimes to fail later and less clearly.

## 9. Further Reading

- [Repository README](../README.md)
- [Chinese upgrade guide](zh-CN/upgrade-from-0.2-to-0.3.md)
- [FFI host checklist](zh-CN/ffi/host-checklist.md)
- [FFI integration guide](zh-CN/ffi/integration-guide.md)
- [Skill development manual](skill-development.md)
