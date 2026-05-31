// Handle one managed Node.js smoke request from Lua.
// 处理一次来自 Lua 的受管 Node.js 冒烟请求。
export async function main(args, ctx) {
  // stdout proves the worker captures Node.js console output.
  // stdout 用于证明 worker 会捕获 Node.js console 输出。
  console.log("node stdout ready");
  // The returned object proves JSON value transport and argument passing.
  // 返回对象用于证明 JSON 值传输与参数传递正常。
  return {
    runtime: "node",
    text: args.text || "",
    number: (args.number || 0) + 2,
    ctxIsObject: ctx && typeof ctx === "object",
  };
}
