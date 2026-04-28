import { LuaSkillsClient, LuaSkillsJsonFfi } from "../dist/index.js";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const exampleDir = dirname(fileURLToPath(import.meta.url));
const libraryPath = process.env.LUASKILLS_LIB ?? resolve(exampleDir, "../../../target/debug/luaskills.dll");
const runtimeRoot = process.env.LUASKILLS_RUNTIME_ROOT ?? resolve(exampleDir, "luaskills-runtime");
const ffi = new LuaSkillsJsonFfi({ libraryPath });

// Return a minimal host-side SQLite provider response for demo requests.
// 为演示请求返回一个最小宿主侧 SQLite provider 响应。
const sqliteProvider = (request) => ({ ok: true, request });

ffi.setSqliteProviderJsonCallback(sqliteProvider);

try {
  const client = LuaSkillsClient.create({
    libraryPath,
    runtimeRoot,
    hostOptions: {
      sqlite_provider_mode: "host_callback",
      sqlite_callback_mode: "json",
    },
  });
  client.close();
  console.log("SQLite JSON provider callback registered before engine creation.");
} finally {
  ffi.clearSqliteProviderJsonCallback();
}
