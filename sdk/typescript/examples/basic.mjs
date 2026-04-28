import { LuaSkillsClient } from "../dist/index.js";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const exampleDir = dirname(fileURLToPath(import.meta.url));
const libraryPath = process.env.LUASKILLS_LIB ?? resolve(exampleDir, "../../../target/debug/luaskills.dll");
const version = LuaSkillsClient.version({ libraryPath });

console.log(version);
