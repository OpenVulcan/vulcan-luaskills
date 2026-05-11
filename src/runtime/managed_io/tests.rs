use super::*;

/// Verify managed read_text decodes GB18030 content.
/// 验证托管 read_text 可以解码 GB18030 内容。
#[test]
fn managed_io_read_text_decodes_gb18030() {
    let lua = Lua::new();
    let io_table =
        create_vulcan_io_table(&lua, RuntimeTextEncoding::Utf8).expect("create vulcan.io");
    let path = std::env::temp_dir().join(format!(
        "luaskills_managed_io_gb18030_{}.txt",
        std::process::id()
    ));
    let bytes =
        encode_runtime_text("中文", RuntimeTextEncoding::Gb18030).expect("encode gb18030 content");
    fs::write(&path, bytes).expect("write test file");
    lua.globals().set("vio", io_table).expect("set io table");
    let script = format!(
        "return vio.read_text({}, {{ encoding = 'gb18030' }})",
        lua_quote(&path.to_string_lossy())
    );
    let value: String = lua.load(&script).eval().expect("read text through Lua");
    assert_eq!(value, "中文");
    let _ = fs::remove_file(path);
}

/// Verify managed read_text uses the table default encoding when options are omitted.
/// 验证托管 read_text 在省略选项时会使用表级默认编码。
#[test]
fn managed_io_read_text_uses_default_encoding() {
    let lua = Lua::new();
    let io_table =
        create_vulcan_io_table(&lua, RuntimeTextEncoding::Gb18030).expect("create vulcan.io");
    let path = std::env::temp_dir().join(format!(
        "luaskills_managed_io_default_gb18030_{}.txt",
        std::process::id()
    ));
    let bytes = encode_runtime_text("默认编码", RuntimeTextEncoding::Gb18030)
        .expect("encode default gb18030 content");
    fs::write(&path, bytes).expect("write default encoding test file");
    lua.globals().set("vio", io_table).expect("set io table");
    let script = format!(
        "return vio.read_text({})",
        lua_quote(&path.to_string_lossy())
    );
    let value: String = lua
        .load(&script)
        .eval()
        .expect("read text through default encoding");
    assert_eq!(value, "默认编码");
    let _ = fs::remove_file(path);
}

/// Verify io compatibility open supports read-all calls.
/// 验证 io 兼容 open 支持读取全部内容。
#[test]
fn managed_io_compat_open_reads_all() {
    let lua = Lua::new();
    let io_table =
        create_vulcan_io_table(&lua, RuntimeTextEncoding::Utf8).expect("create vulcan.io");
    install_managed_io_compat(&lua, &io_table, RuntimeTextEncoding::Utf8)
        .expect("install managed io compat");
    let path = std::env::temp_dir().join(format!(
        "luaskills_managed_io_compat_{}.txt",
        std::process::id()
    ));
    fs::write(&path, "hello").expect("write test file");
    let script = format!(
        "local f = io.open({}, 'r'); local v = f:read('*a'); f:close(); return v",
        lua_quote(&path.to_string_lossy())
    );
    let value: String = lua.load(&script).eval().expect("read through io.open");
    assert_eq!(value, "hello");
    let _ = fs::remove_file(path);
}

/// Verify io.input sets the managed default input used by io.read.
/// 验证 io.input 会设置 io.read 使用的托管默认输入。
#[test]
fn managed_io_compat_input_feeds_read() {
    let lua = Lua::new();
    let io_table =
        create_vulcan_io_table(&lua, RuntimeTextEncoding::Utf8).expect("create vulcan.io");
    install_managed_io_compat(&lua, &io_table, RuntimeTextEncoding::Utf8)
        .expect("install managed io compat");
    let path = std::env::temp_dir().join(format!(
        "luaskills_managed_io_input_{}.txt",
        std::process::id()
    ));
    fs::write(&path, "input-value").expect("write test file");
    let script = format!(
        "io.input({}); return io.read('*a')",
        lua_quote(&path.to_string_lossy())
    );
    let value: String = lua.load(&script).eval().expect("read through io.input");
    assert_eq!(value, "input-value");
    let _ = fs::remove_file(path);
}

/// Verify io.output sets the managed default output used by io.write.
/// 验证 io.output 会设置 io.write 使用的托管默认输出。
#[test]
fn managed_io_compat_output_receives_write() {
    let lua = Lua::new();
    let io_table =
        create_vulcan_io_table(&lua, RuntimeTextEncoding::Utf8).expect("create vulcan.io");
    install_managed_io_compat(&lua, &io_table, RuntimeTextEncoding::Utf8)
        .expect("install managed io compat");
    let path = std::env::temp_dir().join(format!(
        "luaskills_managed_io_output_{}.txt",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let script = format!(
        "io.output({}); io.write('out', '-', 'value'); io.close(); return true",
        lua_quote(&path.to_string_lossy())
    );
    let value: bool = lua.load(&script).eval().expect("write through io.output");
    assert!(value);
    assert_eq!(
        fs::read_to_string(&path).expect("read output file"),
        "out-value"
    );
    let _ = fs::remove_file(path);
}

/// Verify managed io.tmpfile supports write, seek, read, and close.
/// 验证托管 io.tmpfile 支持写入、定位、读取与关闭。
#[test]
fn managed_io_compat_tmpfile_supports_update_reads() {
    let lua = Lua::new();
    let io_table =
        create_vulcan_io_table(&lua, RuntimeTextEncoding::Utf8).expect("create vulcan.io");
    install_managed_io_compat(&lua, &io_table, RuntimeTextEncoding::Utf8)
        .expect("install managed io compat");
    let script = "local f = io.tmpfile(); f:write('tmp-value'); f:seek('set', 0); local value = f:read('*a'); local ok = f:close(); return value, ok";
    let (value, ok): (String, bool) = lua.load(script).eval().expect("use managed tmpfile");
    assert_eq!(value, "tmp-value");
    assert!(ok);
}

/// Verify managed update modes support the common write-seek-read flow.
/// 验证托管更新模式支持常见的写入、回退定位、读取流程。
#[test]
fn managed_io_open_update_mode_supports_seek_read() {
    let lua = Lua::new();
    let io_table =
        create_vulcan_io_table(&lua, RuntimeTextEncoding::Utf8).expect("create vulcan.io");
    install_managed_io_compat(&lua, &io_table, RuntimeTextEncoding::Utf8)
        .expect("install managed io compat");
    let path = std::env::temp_dir().join(format!(
        "luaskills_managed_io_update_{}.txt",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    let script = format!(
        "local f = io.open({}, 'w+'); f:write('update-value'); f:seek('set', 0); local value = f:read('*a'); f:close(); return value",
        lua_quote(&path.to_string_lossy())
    );
    let value: String = lua.load(&script).eval().expect("use managed update mode");
    assert_eq!(value, "update-value");
    assert_eq!(
        fs::read_to_string(&path).expect("read update mode file"),
        "update-value"
    );
    let _ = fs::remove_file(path);
}

/// Quote one Rust string for a compact Lua literal in tests.
/// 为测试生成一个紧凑的 Lua 字符串字面量。
fn lua_quote(value: &str) -> String {
    format!("{:?}", value)
}
