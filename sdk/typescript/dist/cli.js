#!/usr/bin/env node
import { join, resolve } from "node:path";
import { LuaSkillsClient } from "./client.js";
import { RuntimeRoots } from "./roots.js";
import { buildRuntimeInstallManifest, installRuntimeAssets, normalizeDatabasePreset, RuntimeDatabasePreset } from "./runtime-assets.js";
import { Authority, SkillInstallSourceType } from "./types.js";
/**
 * CLI entrypoint that dispatches one command and prints JSON output.
 * CLI 入口，分发单个命令并输出 JSON。
 */
async function main() {
    const parsed = parseArgs(process.argv.slice(2));
    const command = parsed.positionals[0];
    if (!command || command === "help" || command === "--help" || command === "-h") {
        printUsage();
        return;
    }
    const runtimeRoot = resolve(stringFlag(parsed, "runtime-root") ?? join(process.cwd(), "luaskills-runtime"));
    if (command === "version") {
        printJson(LuaSkillsClient.version({ libraryPath: stringFlag(parsed, "lib"), runtimeRoot }));
        return;
    }
    if (command === "describe") {
        printJson(LuaSkillsClient.describe({ libraryPath: stringFlag(parsed, "lib"), runtimeRoot }));
        return;
    }
    if (command === "install-runtime") {
        const installOptions = runtimeInstallOptionsFromArgs(parsed, runtimeRoot);
        printJson(booleanFlag(parsed, "dry-run") ? buildRuntimeInstallManifest(installOptions) : await installRuntimeAssets(installOptions));
        return;
    }
    const skillRoots = rootsFromArgs(parsed, runtimeRoot);
    const client = LuaSkillsClient.create({
        libraryPath: stringFlag(parsed, "lib"),
        runtimeRoot,
        ensureRuntimeLayout: false,
    });
    try {
        if (command !== "load") {
            client.loadFromRoots(skillRoots);
        }
        await dispatchEngineCommand(client, skillRoots, parsed);
    }
    finally {
        client.close();
    }
}
/**
 * Build runtime asset installation options from CLI flags.
 * 从 CLI 标志构造运行时资产安装选项。
 */
function runtimeInstallOptionsFromArgs(parsed, runtimeRoot) {
    return {
        runtimeRoot,
        database: normalizeDatabasePreset(stringFlag(parsed, "database") ?? RuntimeDatabasePreset.None),
        luaskillsVersion: stringFlag(parsed, "luaskills-version"),
        vldbControllerVersion: stringFlag(parsed, "vldb-controller-version"),
        vldbSqliteVersion: stringFlag(parsed, "vldb-sqlite-version"),
        vldbLancedbVersion: stringFlag(parsed, "vldb-lancedb-version"),
        includeLuaSkillsFfi: !booleanFlag(parsed, "skip-luaskills-ffi"),
        luaskillsRepo: stringFlag(parsed, "luaskills-repo"),
        vldbControllerRepo: stringFlag(parsed, "vldb-controller-repo"),
        vldbSqliteRepo: stringFlag(parsed, "vldb-sqlite-repo"),
        vldbLancedbRepo: stringFlag(parsed, "vldb-lancedb-repo"),
    };
}
/**
 * Dispatch one command that requires an engine handle.
 * 分发一个需要引擎句柄的命令。
 */
async function dispatchEngineCommand(client, skillRoots, parsed) {
    const [command, first, second, third, fourth] = parsed.positionals;
    const authority = authorityFlag(parsed);
    switch (command) {
        case "load":
            printJson(client.loadFromRoots(skillRoots));
            return;
        case "reload":
            printJson(client.reloadFromRoots(skillRoots));
            return;
        case "list":
            printJson(client.listEntries(authority));
            return;
        case "help-list":
            printJson(client.listSkillHelp(authority));
            return;
        case "help-detail":
            requireValue(first, "help-detail requires <skill-id>");
            printJson(client.renderSkillHelpDetail(first, second ?? "main", { authority }));
            return;
        case "is-skill":
            requireValue(first, "is-skill requires <tool-name>");
            printJson({ value: client.isSkill(first, authority) });
            return;
        case "skill-name":
            requireValue(first, "skill-name requires <tool-name>");
            printJson({ skill_id: client.skillNameForTool(first, authority) });
            return;
        case "prompt-completions":
            requireValue(first, "prompt-completions requires <prompt-name> <argument-name>");
            requireValue(second, "prompt-completions requires <prompt-name> <argument-name>");
            printJson(client.promptArgumentCompletions(first, second, authority));
            return;
        case "call":
            requireValue(first, "call requires <tool-name> [args-json]");
            printJson(client.callSkill(first, parseJsonValue(second ?? stringFlag(parsed, "args") ?? "{}")));
            return;
        case "run-lua":
            printJson(client.runLua(stringFlag(parsed, "code") ?? requireValue(first, "run-lua requires <code> [args-json]"), parseJsonValue(second ?? stringFlag(parsed, "args") ?? "{}")));
            return;
        case "config":
            dispatchConfigCommand(client, first, second, third, fourth);
            return;
        case "disable":
            printJson(client.skills.disable(skillRoots, requireValue(first, "disable requires <skill-id>"), second ?? null));
            return;
        case "enable":
            printJson(client.skills.enable(skillRoots, requireValue(first, "enable requires <skill-id>")));
            return;
        case "install":
            printJson(client.skills.install(skillRoots, installRequestFromArgs(parsed), lifecycleOptionsFromArgs(parsed, skillRoots)));
            return;
        case "update":
            printJson(client.skills.update(skillRoots, installRequestFromArgs(parsed), lifecycleOptionsFromArgs(parsed, skillRoots)));
            return;
        case "uninstall":
            printJson(client.skills.uninstall(skillRoots, requireValue(first, "uninstall requires <skill-id>"), uninstallOptionsFromArgs(parsed), lifecycleOptionsFromArgs(parsed, skillRoots)));
            return;
        case "system-disable":
            printJson(client.system(authority).disable(skillRoots, requireValue(first, "system-disable requires <skill-id>"), second ?? null));
            return;
        case "system-enable":
            printJson(client.system(authority).enable(skillRoots, requireValue(first, "system-enable requires <skill-id>")));
            return;
        case "system-install":
            printJson(client.system(authority).install(skillRoots, installRequestFromArgs(parsed), lifecycleOptionsFromArgs(parsed, skillRoots)));
            return;
        case "system-update":
            printJson(client.system(authority).update(skillRoots, installRequestFromArgs(parsed), lifecycleOptionsFromArgs(parsed, skillRoots)));
            return;
        case "system-uninstall":
            printJson(client.system(authority).uninstall(skillRoots, requireValue(first, "system-uninstall requires <skill-id>"), uninstallOptionsFromArgs(parsed), lifecycleOptionsFromArgs(parsed, skillRoots)));
            return;
        default:
            throw new Error(`Unknown luaskills command: ${command}`);
    }
}
/**
 * Dispatch one skill-config subcommand.
 * 分发单个 skill-config 子命令。
 */
function dispatchConfigCommand(client, subcommand, first, second, third) {
    switch (subcommand) {
        case "list":
            printJson(client.config.list(first));
            return;
        case "get":
            printJson(client.config.get(requireValue(first, "config get requires <skill-id> <key>"), requireValue(second, "config get requires <skill-id> <key>")));
            return;
        case "set":
            printJson(client.config.set(requireValue(first, "config set requires <skill-id> <key> <value>"), requireValue(second, "config set requires <skill-id> <key> <value>"), requireValue(third, "config set requires <skill-id> <key> <value>")));
            return;
        case "delete":
            printJson(client.config.delete(requireValue(first, "config delete requires <skill-id> <key>"), requireValue(second, "config delete requires <skill-id> <key>")));
            return;
        default:
            throw new Error("config requires one of: list, get, set, delete");
    }
}
/**
 * Parse raw command-line arguments into positionals and flags.
 * 将原始命令行参数解析为位置参数和标志。
 */
function parseArgs(rawArgs) {
    const positionals = [];
    const flags = new Map();
    for (let index = 0; index < rawArgs.length; index += 1) {
        const value = rawArgs[index] ?? "";
        if (!value.startsWith("--")) {
            positionals.push(value);
            continue;
        }
        const trimmed = value.slice(2);
        const equalsIndex = trimmed.indexOf("=");
        if (equalsIndex >= 0) {
            flags.set(trimmed.slice(0, equalsIndex), trimmed.slice(equalsIndex + 1));
            continue;
        }
        const next = rawArgs[index + 1];
        if (next && !next.startsWith("--")) {
            flags.set(trimmed, next);
            index += 1;
        }
        else {
            flags.set(trimmed, true);
        }
    }
    return { positionals, flags };
}
/**
 * Build the formal root chain from CLI flags.
 * 从 CLI 标志构造正式 root 链。
 */
function rootsFromArgs(parsed, runtimeRoot) {
    const roots = RuntimeRoots.standard({
        runtimeRoot,
        includeProject: !booleanFlag(parsed, "no-project") && !booleanFlag(parsed, "root-only"),
        includeUser: !booleanFlag(parsed, "no-user") && !booleanFlag(parsed, "root-only"),
    });
    replaceRootDir(roots, "ROOT", stringFlag(parsed, "root-skills"));
    replaceRootDir(roots, "PROJECT", stringFlag(parsed, "project-skills"));
    replaceRootDir(roots, "USER", stringFlag(parsed, "user-skills"));
    RuntimeRoots.ensureLayout(runtimeRoot, roots);
    return roots;
}
/**
 * Replace one configured root directory when the root exists and a value is present.
 * 当 root 存在且传入值存在时替换单个已配置 root 目录。
 */
function replaceRootDir(roots, label, value) {
    if (!value) {
        return;
    }
    const root = roots.find((candidate) => candidate.name.trim().toUpperCase() === label);
    if (root) {
        root.skills_dir = resolve(value);
    }
}
/**
 * Build one install or update request from CLI flags.
 * 从 CLI 标志构造单个安装或更新请求。
 */
function installRequestFromArgs(parsed) {
    const source = stringFlag(parsed, "source") ?? parsed.positionals[1] ?? null;
    const sourceType = stringFlag(parsed, "source-type") ?? SkillInstallSourceType.Github;
    return {
        skill_id: stringFlag(parsed, "skill-id") ?? null,
        source,
        source_type: sourceType,
    };
}
/**
 * Build lifecycle options from CLI flags.
 * 从 CLI 标志构造生命周期选项。
 */
function lifecycleOptionsFromArgs(parsed, skillRoots) {
    const targetRootLabel = stringFlag(parsed, "target-root");
    return {
        targetRoot: targetRootLabel ? RuntimeRoots.findByLabel(skillRoots, targetRootLabel) : undefined,
        authority: authorityFlag(parsed),
    };
}
/**
 * Build uninstall cleanup options from CLI flags.
 * 从 CLI 标志构造卸载清理选项。
 */
function uninstallOptionsFromArgs(parsed) {
    return {
        remove_sqlite: booleanFlag(parsed, "remove-sqlite"),
        remove_lancedb: booleanFlag(parsed, "remove-lancedb"),
    };
}
/**
 * Return one string flag value when present.
 * 返回存在的单个字符串标志值。
 */
function stringFlag(parsed, name) {
    const value = parsed.flags.get(name);
    return typeof value === "string" ? value : undefined;
}
/**
 * Return one boolean flag value.
 * 返回单个布尔标志值。
 */
function booleanFlag(parsed, name) {
    return parsed.flags.get(name) === true;
}
/**
 * Return the selected host-injected authority.
 * 返回选中的宿主注入权限。
 */
function authorityFlag(parsed) {
    const authority = stringFlag(parsed, "authority") ?? Authority.DelegatedTool;
    if (authority !== Authority.System && authority !== Authority.DelegatedTool) {
        throw new Error("--authority must be 'system' or 'delegated_tool'");
    }
    return authority;
}
/**
 * Parse one JSON value from CLI text.
 * 从 CLI 文本解析单个 JSON 值。
 */
function parseJsonValue(text) {
    return JSON.parse(text);
}
/**
 * Require one positional value and return it.
 * 要求单个位置参数存在并返回该值。
 */
function requireValue(value, message) {
    if (!value) {
        throw new Error(message);
    }
    return value;
}
/**
 * Print one value as pretty JSON.
 * 将单个值以美化 JSON 输出。
 */
function printJson(value) {
    process.stdout.write(`${JSON.stringify(value, null, 2)}\n`);
}
/**
 * Print concise CLI usage text.
 * 输出简洁 CLI 用法文本。
 */
function printUsage() {
    process.stdout.write(`luaskills <command> [options]

Commands:
  version
  describe
  install-runtime [--database none|vldb-controller|vldb-direct|host-callback]
  load | reload | list
  help-list | help-detail <skill-id> [flow]
  is-skill <tool-name> | skill-name <tool-name>
  prompt-completions <prompt-name> <argument-name>
  call <tool-name> [args-json]
  run-lua <code> [args-json]
  config list [skill-id]
  config get <skill-id> <key>
  config set <skill-id> <key> <value>
  config delete <skill-id> <key>
  install|update <source> [--skill-id id] [--target-root USER]
  uninstall <skill-id> [--target-root USER] [--remove-sqlite] [--remove-lancedb]
  system-install|system-update|system-uninstall ... [--authority system]
  system-enable|system-disable <skill-id> [--authority system]

Global options:
  --lib <path>              LuaSkills dynamic library path, or LUASKILLS_LIB
  --runtime-root <path>     Shared runtime root
  --authority <value>       delegated_tool or system
  --root-only               Use only ROOT root
  --target-root <label>     ROOT, PROJECT, or USER
  --dry-run                 Print the runtime asset plan without downloading
  --skip-luaskills-ffi      Do not install the luaskills FFI SDK archive
`);
}
main().catch((error) => {
    const message = error instanceof Error ? error.message : String(error);
    process.stderr.write(`${message}\n`);
    process.exitCode = 1;
});
//# sourceMappingURL=cli.js.map