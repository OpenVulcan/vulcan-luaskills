"""
Minimal Python host demo for one persistent runtime session lease.
演示单个持久运行时会话租约的最小 Python 宿主示例。
"""

from demo import load_library, standard_fixture_runtime_root
from json_runtime import (
    JsonFfiClient,
    SKILL_AUTHORITY_SYSTEM,
    StandardFixtureRuntimeClient,
)


RUNTIME_SESSION_SID = "python-runtime-session-demo"


def main() -> None:
    """
    Demonstrate one persistent runtime lease that reuses one interactive child-process handle across multiple eval calls.
    演示一个持久运行时租约如何在多次 eval 调用之间复用同一个交互式子进程句柄。
    """

    runtime_root = standard_fixture_runtime_root()
    client = JsonFfiClient(load_library())
    runtime = StandardFixtureRuntimeClient(client, runtime_root)

    engine_id = runtime.create_engine(
        default_text_encoding="utf-8",
        enable_managed_io_compat=True,
    )
    print("Engine created:", engine_id)

    try:
        runtime.load_root(engine_id)
        print("Loaded roots from:", runtime_root / "skills")

        system = runtime.system_client(
            engine_id,
            authority=SKILL_AUTHORITY_SYSTEM,
        )
        print("Visible entry count:", len(system.list_entries()))
        print(
            "Visible skill ownership:",
            system.skill_name_for_tool("demo-standard-ffi-skill-ping"),
        )

        sessions = system.runtime_sessions()
        print(
            "Uses dedicated system runtime-session endpoints:",
            sessions.uses_system_runtime_session_endpoints(),
        )
        session = sessions.create_handle(RUNTIME_SESSION_SID, ttl_sec=600, replace=True)
        identity = session.identity_payload()
        lease_id = str(identity["lease_id"])
        print("Lease created:", lease_id)
        print("Lease handle count:", len(sessions.list_handles(RUNTIME_SESSION_SID)))

        opened = session.eval(
            """
local info = vulcan.os.info()
if not proc then
  local spec
  if info.os == "windows" then
    spec = {
      program = "cmd",
      args = { "/V:ON", "/C", "set /P line=&echo session:!line!" },
      encoding = "utf-8",
    }
  else
    spec = {
      program = "sh",
      args = { "-c", "read line; echo session:$line" },
      encoding = "utf-8",
    }
  end
  proc = vulcan.process.session.open(spec)
end
counter = (counter or 0) + 1
proc:write((args.input or "runtime-session-demo") .. "\\n")
return {
  opened = true,
  counter = counter,
  input = args.input,
}
""",
            args={"input": "runtime-session-demo"},
        )
        print("Open eval result:", opened["result"])

        read_output = session.eval(
            """
counter = (counter or 0) + 1
local output = proc:read({ timeout_ms = 2000, max_bytes = 8192 })
return {
  counter = counter,
  stdout = output.stdout,
  stderr = output.stderr,
  timed_out = output.timed_out,
}
""",
        )
        print("Read eval result:", read_output["result"])

        current_status = session.status()
        print("Lease status result:", current_status)

        closed_process = session.eval(
            """
counter = (counter or 0) + 1
local status = proc:close({ timeout_ms = 3000 })
proc = nil
return {
  counter = counter,
  exited = status.exited,
  success = status.success,
}
""",
        )
        print("Close process eval result:", closed_process["result"])

        closed_lease = session.close()
        print("Lease close result:", closed_lease)

        post_close = sessions.call_raw(
            "eval",
            {
                "lease_id": lease_id,
                "sid": identity["sid"],
                "generation": identity["generation"],
                "timeout_ms": 60_000,
                "args": {},
                "code": "return 1",
            },
        )
        print("Post-close eval result:", post_close)
    finally:
        runtime.free_engine(engine_id)
        print("Engine freed")


if __name__ == "__main__":
    main()
