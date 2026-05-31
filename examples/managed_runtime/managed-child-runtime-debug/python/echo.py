# Handle one managed Python smoke request from Lua.
# 处理一次来自 Lua 的受管 Python 冒烟请求。
def main(args, ctx):
    # stdout proves the worker captures Python standard output.
    # stdout 用于证明 worker 会捕获 Python 标准输出。
    print("python stdout ready")
    # The returned object proves JSON value transport and argument passing.
    # 返回对象用于证明 JSON 值传输与参数传递正常。
    return {
        "runtime": "python",
        "text": args.get("text", ""),
        "number": args.get("number", 0) + 1,
        "ctx_is_dict": isinstance(ctx, dict),
    }
