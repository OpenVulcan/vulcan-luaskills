use super::*;
use crate::host::callbacks::{
    RuntimeModelCaller, dispatch_model_embed_request, dispatch_model_llm_request,
    runtime_model_callback_test_guard,
};
use crate::runtime_help::{
    RuntimeHelpDetail as RuntimeHelpDetailModel,
    RuntimeHelpNodeDescriptor as RuntimeHelpNodeDescriptorModel,
    RuntimeSkillHelpDescriptor as RuntimeSkillHelpDescriptorModel,
};
use crate::{
    RuntimeEntryDescriptor as RuntimeEntryDescriptorModel,
    RuntimeEntryParameterDescriptor as RuntimeEntryParameterDescriptorModel,
};

/// Read one owned UTF-8 buffer into one Rust string without freeing it.
/// 将一个拥有型 UTF-8 缓冲读取为 Rust 字符串但不执行释放。
fn read_owned_buffer_text(buffer: &FfiOwnedBuffer) -> String {
    if buffer.ptr.is_null() || buffer.len == 0 {
        return String::new();
    }
    let bytes = unsafe { std::slice::from_raw_parts(buffer.ptr, buffer.len) };
    String::from_utf8(bytes.to_vec()).expect("buffer text must be utf-8")
}

/// Build one borrowed buffer view over one UTF-8 text while keeping backing storage alive.
/// 在保持底层存储存活的前提下，为一段 UTF-8 文本构造借用缓冲视图。
fn make_borrowed_buffer(text: &str) -> (Vec<u8>, FfiBorrowedBuffer) {
    let bytes = text.as_bytes().to_vec();
    let buffer = if bytes.is_empty() {
        FfiBorrowedBuffer {
            ptr: ptr::null(),
            len: 0,
        }
    } else {
        FfiBorrowedBuffer {
            ptr: bytes.as_ptr(),
            len: bytes.len(),
        }
    };
    (bytes, buffer)
}

/// Verify buffer_clone copies one byte payload into luaskills-owned storage.
/// 验证 buffer_clone 会把单个字节载荷复制到 luaskills 自主管理存储中。
#[test]
fn buffer_clone_copies_payload_into_owned_storage() {
    let input = b"ffi-buffer-demo";
    let mut buffer_out = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let mut error_out = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let status = unsafe {
        luaskills_ffi_buffer_clone(input.as_ptr(), input.len(), &mut buffer_out, &mut error_out)
    };
    assert_eq!(status, FFI_STATUS_OK);
    assert!(error_out.ptr.is_null());
    assert_eq!(error_out.len, 0);
    let copied = unsafe { std::slice::from_raw_parts(buffer_out.ptr, buffer_out.len) };
    assert_eq!(copied, input);
    unsafe { luaskills_ffi_buffer_free(buffer_out) };
}

/// Verify JSON provider callback bridge accepts borrowed buffers and owned-buffer responses.
/// 验证 JSON provider callback 桥接可接受借用缓冲输入并处理拥有型缓冲输出。
#[test]
fn json_provider_callback_bridge_round_trips_owned_buffers() {
    unsafe extern "C" fn callback(
        request_json: FfiBorrowedBuffer,
        _user_data: *mut c_void,
        response_out: *mut FfiOwnedBuffer,
        error_out: *mut FfiOwnedBuffer,
    ) -> i32 {
        let request_bytes =
            unsafe { std::slice::from_raw_parts(request_json.ptr, request_json.len) };
        let request_text = std::str::from_utf8(request_bytes).expect("request must be utf-8");
        let response_text = format!("{{\"echo\":{}}}", request_text);
        unsafe {
            *response_out = alloc_owned_buffer_from_bytes(response_text.as_bytes());
            *error_out = FfiOwnedBuffer {
                ptr: ptr::null_mut(),
                len: 0,
            };
        }
        FFI_STATUS_OK
    }

    let response = invoke_json_provider_callback(callback, 0, "{\"value\":1}")
        .expect("callback bridge should succeed");
    assert_eq!(response, "{\"echo\":{\"value\":1}}");
}

/// Verify model JSON FFI callback setters round-trip requests, responses, and provider error fields.
/// 验证模型 JSON FFI callback setter 会往返传递请求、响应和 provider 错误字段。
#[test]
fn model_json_callback_setters_round_trip_response_and_provider_error() {
    unsafe extern "C" fn embed_callback(
        request_json: FfiBorrowedBuffer,
        _user_data: *mut c_void,
        response_out: *mut FfiOwnedBuffer,
        error_out: *mut FfiOwnedBuffer,
    ) -> i32 {
        let request_bytes =
            unsafe { std::slice::from_raw_parts(request_json.ptr, request_json.len) };
        let request: Value = match serde_json::from_slice(request_bytes) {
            Ok(request) => request,
            Err(error) => {
                unsafe {
                    *error_out =
                        alloc_owned_buffer_from_string(format!("invalid request: {}", error));
                }
                return FFI_STATUS_ERROR;
            }
        };
        if request["text"] != "hello"
            || request["caller"]["skill_id"] != "ffi-skill"
            || request["caller"]["request_id"] != "req-ffi-1"
        {
            unsafe {
                *error_out =
                    alloc_owned_buffer_from_string(format!("unexpected request: {}", request));
            }
            return FFI_STATUS_ERROR;
        }
        unsafe {
            *response_out = alloc_owned_buffer_from_string(
                r#"{"ok":true,"vector":[0.1,0.2],"dimensions":2,"usage":{"input_tokens":3}}"#,
            );
            *error_out = FfiOwnedBuffer {
                ptr: ptr::null_mut(),
                len: 0,
            };
        }
        FFI_STATUS_OK
    }

    unsafe extern "C" fn llm_callback(
        request_json: FfiBorrowedBuffer,
        _user_data: *mut c_void,
        response_out: *mut FfiOwnedBuffer,
        error_out: *mut FfiOwnedBuffer,
    ) -> i32 {
        let request_bytes =
            unsafe { std::slice::from_raw_parts(request_json.ptr, request_json.len) };
        let request: Value = match serde_json::from_slice(request_bytes) {
            Ok(request) => request,
            Err(error) => {
                unsafe {
                    *error_out =
                        alloc_owned_buffer_from_string(format!("invalid request: {}", error));
                }
                return FFI_STATUS_ERROR;
            }
        };
        if request["system"] != "system" || request["user"] != "user" {
            unsafe {
                *error_out =
                    alloc_owned_buffer_from_string(format!("unexpected request: {}", request));
            }
            return FFI_STATUS_ERROR;
        }
        unsafe {
            *response_out = alloc_owned_buffer_from_string(
                r#"{"ok":false,"error":{"code":"provider_error","message":"provider failed","provider_message":"raw provider message","provider_code":"invalid_api_key","provider_status":401}}"#,
            );
            *error_out = FfiOwnedBuffer {
                ptr: ptr::null_mut(),
                len: 0,
            };
        }
        FFI_STATUS_OK
    }

    let _guard = runtime_model_callback_test_guard();
    let exported = crate::ffi::exported_ffi_function_names();
    assert!(
        exported
            .iter()
            .any(|name| name == "luaskills_ffi_set_model_embed_json_callback")
    );
    assert!(
        exported
            .iter()
            .any(|name| name == "luaskills_ffi_set_model_llm_json_callback")
    );

    let mut error_out = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let embed_status = unsafe {
        luaskills_ffi_set_model_embed_json_callback(
            Some(embed_callback),
            ptr::null_mut(),
            &mut error_out,
        )
    };
    assert_eq!(embed_status, FFI_STATUS_OK);
    assert!(error_out.ptr.is_null());
    let llm_status = unsafe {
        luaskills_ffi_set_model_llm_json_callback(
            Some(llm_callback),
            ptr::null_mut(),
            &mut error_out,
        )
    };
    assert_eq!(llm_status, FFI_STATUS_OK);
    assert!(error_out.ptr.is_null());

    let caller = RuntimeModelCaller {
        skill_id: Some("ffi-skill".to_string()),
        entry_name: Some("entry".to_string()),
        canonical_tool_name: Some("ffi-skill-entry".to_string()),
        root_name: Some("ROOT".to_string()),
        skill_dir: Some("D:/skills/ffi-skill".to_string()),
        client_name: Some("sdk-test".to_string()),
        request_id: Some("req-ffi-1".to_string()),
    };
    let embed_response = dispatch_model_embed_request(&RuntimeModelEmbedRequest {
        text: "hello".to_string(),
        caller: caller.clone(),
    })
    .expect("embed JSON callback should return a response");
    assert_eq!(embed_response.vector, vec![0.1, 0.2]);
    assert_eq!(embed_response.dimensions, 2);
    assert_eq!(
        embed_response.usage.and_then(|usage| usage.input_tokens),
        Some(3)
    );

    let llm_error = dispatch_model_llm_request(&RuntimeModelLlmRequest {
        system: "system".to_string(),
        user: "user".to_string(),
        caller,
    })
    .expect_err("llm JSON callback should return a provider error");
    assert_eq!(llm_error.code, RuntimeModelErrorCode::ProviderError);
    assert_eq!(llm_error.message, "provider failed");
    assert_eq!(
        llm_error.provider_message.as_deref(),
        Some("raw provider message")
    );
    assert_eq!(llm_error.provider_code.as_deref(), Some("invalid_api_key"));
    assert_eq!(llm_error.provider_status, Some(401));
}

/// Verify one entry list allocates nested owned buffers for entry and parameter text fields.
/// 验证入口列表会为入口及参数文本字段分配嵌套拥有型缓冲。
#[test]
fn entry_list_free_handles_nested_owned_buffers() {
    let runtime_entry = RuntimeEntryDescriptorModel {
        canonical_name: "demo-entry".to_string(),
        skill_id: "demo-skill".to_string(),
        local_name: "entry".to_string(),
        root_name: "ROOT".to_string(),
        skill_dir: "/tmp/demo-skill".to_string(),
        description: "Demo entry description".to_string(),
        parameters: vec![RuntimeEntryParameterDescriptorModel {
            name: "note".to_string(),
            param_type: "string".to_string(),
            description: "Optional note".to_string(),
            required: false,
        }],
    };

    let mut items = vec![alloc_entry_descriptor(&runtime_entry)];
    let list = FfiRuntimeEntryDescriptorList {
        items: items.as_mut_ptr(),
        len: items.len(),
    };
    std::mem::forget(items);
    let list_ptr = Box::into_raw(Box::new(list));

    let list_ref = unsafe { &*list_ptr };
    assert_eq!(list_ref.len, 1);
    let first_entry = unsafe { &*list_ref.items };
    assert_eq!(
        read_owned_buffer_text(&first_entry.canonical_name),
        "demo-entry"
    );
    assert_eq!(read_owned_buffer_text(&first_entry.skill_id), "demo-skill");
    assert_eq!(
        read_owned_buffer_text(&first_entry.description),
        "Demo entry description"
    );
    assert_eq!(first_entry.parameters_len, 1);

    let first_parameter = unsafe { &*first_entry.parameters };
    assert_eq!(read_owned_buffer_text(&first_parameter.name), "note");
    assert_eq!(
        read_owned_buffer_text(&first_parameter.param_type),
        "string"
    );
    assert_eq!(
        read_owned_buffer_text(&first_parameter.description),
        "Optional note"
    );
    assert_eq!(first_parameter.required, 0);

    unsafe { luaskills_ffi_entry_list_free(list_ptr) };
}

/// Verify one help detail and one help list allocate nested owned buffers for text and related-entry arrays.
/// 验证帮助详情与帮助列表会为文本字段和关联入口数组分配嵌套拥有型缓冲。
#[test]
fn help_results_free_handle_nested_owned_buffers() {
    let help_detail = RuntimeHelpDetailModel {
        skill_id: "demo-skill".to_string(),
        skill_name: "Demo Skill".to_string(),
        skill_version: "0.1.0".to_string(),
        root_name: "ROOT".to_string(),
        skill_dir: "/tmp/demo-skill".to_string(),
        flow_name: "main".to_string(),
        description: "Demo help detail".to_string(),
        related_entries: vec!["demo-entry".to_string(), "demo-entry-2".to_string()],
        is_main: true,
        content_type: "markdown".to_string(),
        content: "# Demo".to_string(),
    };
    let detail_ptr = Box::into_raw(Box::new(alloc_help_detail(&help_detail)));

    let detail_ref = unsafe { &*detail_ptr };
    assert_eq!(read_owned_buffer_text(&detail_ref.skill_id), "demo-skill");
    assert_eq!(read_owned_buffer_text(&detail_ref.flow_name), "main");
    assert_eq!(detail_ref.related_entries_len, 2);
    let related_entries = unsafe {
        std::slice::from_raw_parts(detail_ref.related_entries, detail_ref.related_entries_len)
    };
    assert_eq!(read_owned_buffer_text(&related_entries[0]), "demo-entry");
    assert_eq!(read_owned_buffer_text(&related_entries[1]), "demo-entry-2");

    unsafe { luaskills_ffi_help_detail_free(detail_ptr) };

    let help_descriptor = RuntimeSkillHelpDescriptorModel {
        skill_id: "demo-skill".to_string(),
        skill_name: "Demo Skill".to_string(),
        skill_version: "0.1.0".to_string(),
        root_name: "ROOT".to_string(),
        skill_dir: "/tmp/demo-skill".to_string(),
        main: RuntimeHelpNodeDescriptorModel {
            flow_name: "main".to_string(),
            description: "Main help node".to_string(),
            related_entries: vec!["demo-entry".to_string()],
            is_main: true,
        },
        flows: vec![RuntimeHelpNodeDescriptorModel {
            flow_name: "secondary".to_string(),
            description: "Secondary node".to_string(),
            related_entries: vec!["demo-entry-2".to_string()],
            is_main: false,
        }],
    };

    let mut items = vec![alloc_help_descriptor(&help_descriptor)];
    let list = FfiRuntimeSkillHelpDescriptorList {
        items: items.as_mut_ptr(),
        len: items.len(),
    };
    std::mem::forget(items);
    let list_ptr = Box::into_raw(Box::new(list));

    let list_ref = unsafe { &*list_ptr };
    assert_eq!(list_ref.len, 1);
    let first_help = unsafe { &*list_ref.items };
    assert_eq!(read_owned_buffer_text(&first_help.skill_name), "Demo Skill");
    assert_eq!(read_owned_buffer_text(&first_help.main.flow_name), "main");
    assert_eq!(first_help.main.related_entries_len, 1);
    let main_related_entries = unsafe {
        std::slice::from_raw_parts(
            first_help.main.related_entries,
            first_help.main.related_entries_len,
        )
    };
    assert_eq!(
        read_owned_buffer_text(&main_related_entries[0]),
        "demo-entry"
    );
    assert_eq!(first_help.flows_len, 1);
    let first_flow = unsafe { &*first_help.flows };
    assert_eq!(read_owned_buffer_text(&first_flow.flow_name), "secondary");

    unsafe { luaskills_ffi_help_list_free(list_ptr) };
}

/// Verify the standard FFI load/list pipeline returns one entry for one minimal temporary skill root.
/// 验证标准 FFI 的加载与列举链路会为最小临时技能根返回一个入口。
#[test]
fn standard_ffi_load_and_list_entries_round_trip() {
    let temp_root = std::env::temp_dir().join(format!(
        "luaskills_standard_ffi_entry_test_{}",
        std::process::id()
    ));
    if temp_root.exists() {
        let _ = std::fs::remove_dir_all(&temp_root);
    }

    let root_skills_root = temp_root.join("root_skills");
    let skills_root = temp_root.join("skills");
    let skill_dir = skills_root.join("demo-skill");
    std::fs::create_dir_all(&root_skills_root).expect("create root skills root");
    std::fs::create_dir_all(skill_dir.join("runtime")).expect("create runtime directory");
    std::fs::create_dir_all(temp_root.join("temp")).expect("create temp directory");
    std::fs::create_dir_all(temp_root.join("resources")).expect("create resources directory");
    std::fs::create_dir_all(temp_root.join("lua_packages")).expect("create lua_packages directory");
    std::fs::create_dir_all(temp_root.join("bin").join("tools")).expect("create tools directory");
    std::fs::create_dir_all(temp_root.join("libs")).expect("create libs directory");
    std::fs::write(
            skill_dir.join("skill.yaml"),
            "name: demo-skill\nversion: 0.1.0\nenable: true\nentries:\n  - name: ping\n    description: Ping entry.\n    lua_entry: runtime/ping.lua\n    lua_module: demo_skill_ping\n    parameters:\n      - name: note\n        type: string\n        description: Optional note.\n        required: false\n",
        )
        .expect("write skill yaml");
    std::fs::write(
        skill_dir.join("runtime").join("ping.lua"),
        "return function(args)\n  return 'ok'\nend\n",
    )
    .expect("write runtime lua");

    let temp_dir_text =
        CString::new(temp_root.join("temp").display().to_string()).expect("temp_dir cstring");
    let resources_dir_text = CString::new(temp_root.join("resources").display().to_string())
        .expect("resources_dir cstring");
    let lua_packages_dir_text = CString::new(temp_root.join("lua_packages").display().to_string())
        .expect("lua_packages_dir cstring");
    let tool_root_dir_text =
        CString::new(temp_root.join("bin").join("tools").display().to_string())
            .expect("tool_root_dir cstring");
    let ffi_root_dir_text =
        CString::new(temp_root.join("libs").display().to_string()).expect("ffi_root cstring");
    let dependency_dir_name = CString::new("dependencies").expect("dependencies cstring");
    let state_dir_name = CString::new("state").expect("state cstring");
    let database_dir_name = CString::new("databases").expect("databases cstring");
    let root_name = CString::new(" ROOT ").expect("root name cstring");
    let skills_root_text =
        CString::new(skills_root.display().to_string()).expect("skills root cstring");
    let tool_name = CString::new("demo-skill-ping").expect("tool name cstring");

    let host_options = FfiLuaRuntimeHostOptions {
        temp_dir: temp_dir_text.as_ptr(),
        resources_dir: resources_dir_text.as_ptr(),
        lua_packages_dir: lua_packages_dir_text.as_ptr(),
        host_provided_tool_root: tool_root_dir_text.as_ptr(),
        host_provided_lua_root: lua_packages_dir_text.as_ptr(),
        host_provided_ffi_root: ffi_root_dir_text.as_ptr(),
        system_lua_lib_dir: ptr::null(),
        download_cache_root: ptr::null(),
        dependency_dir_name: dependency_dir_name.as_ptr(),
        state_dir_name: state_dir_name.as_ptr(),
        database_dir_name: database_dir_name.as_ptr(),
        skill_config_file_path: ptr::null(),
        allow_network_download: 0,
        github_base_url: ptr::null(),
        github_api_base_url: ptr::null(),
        official_skill_hub_base_url: ptr::null(),
        enable_private_url_skill_install: 0,
        private_skill_source_allowlist: ptr::null(),
        private_skill_source_allowlist_len: 0,
        sqlite_library_path: ptr::null(),
        sqlite_provider_mode: FFI_PROVIDER_MODE_DYNAMIC_LIBRARY,
        sqlite_callback_mode: FFI_CALLBACK_MODE_STANDARD,
        lancedb_library_path: ptr::null(),
        lancedb_provider_mode: FFI_PROVIDER_MODE_DYNAMIC_LIBRARY,
        lancedb_callback_mode: FFI_CALLBACK_MODE_STANDARD,
        space_controller_endpoint: ptr::null(),
        space_controller_auto_spawn: 0,
        space_controller_executable_path: ptr::null(),
        space_controller_process_mode: FFI_SPACE_CONTROLLER_PROCESS_MODE_SERVICE,
        cache_config: ptr::null(),
        runlua_pool_config: ptr::null(),
        reserved_entry_names: ptr::null(),
        reserved_entry_names_len: 0,
        ignored_skill_ids: ptr::null(),
        ignored_skill_ids_len: 0,
        enable_skill_management_bridge: 0,
        default_text_encoding: ptr::null(),
        disable_managed_io_compat: 0,
    };
    let engine_options = FfiLuaEngineOptions {
        pool: FfiLuaVmPoolConfig {
            min_size: 1,
            max_size: 1,
            idle_ttl_secs: 30,
        },
        host: host_options,
    };

    let mut engine_id = 0_u64;
    let mut error_out = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let engine_status =
        unsafe { luaskills_ffi_engine_new(&engine_options, &mut engine_id, &mut error_out) };
    assert_eq!(engine_status, FFI_STATUS_OK);
    assert!(error_out.ptr.is_null());

    let ffi_skill_roots = [FfiRuntimeSkillRoot {
        name: root_name.as_ptr(),
        skills_dir: skills_root_text.as_ptr(),
    }];
    let mut load_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let load_status = unsafe {
        luaskills_ffi_load_from_roots(
            engine_id,
            ffi_skill_roots.as_ptr(),
            ffi_skill_roots.len(),
            &mut load_error,
        )
    };
    assert_eq!(load_status, FFI_STATUS_OK);
    assert!(load_error.ptr.is_null());

    let mut entries_out: *mut FfiRuntimeEntryDescriptorList = ptr::null_mut();
    let mut list_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let list_status = unsafe {
        luaskills_ffi_list_entries(
            engine_id,
            FFI_SKILL_AUTHORITY_SYSTEM,
            &mut entries_out,
            &mut list_error,
        )
    };
    assert_eq!(list_status, FFI_STATUS_OK);
    assert!(list_error.ptr.is_null());
    assert!(!entries_out.is_null());

    let entries_ref = unsafe { &*entries_out };
    assert_eq!(entries_ref.len, 1);
    let entry_ref = unsafe { &*entries_ref.items };
    assert_eq!(
        read_owned_buffer_text(&entry_ref.canonical_name),
        "demo-skill-ping"
    );
    assert_eq!(read_owned_buffer_text(&entry_ref.skill_id), "demo-skill");
    assert_eq!(read_owned_buffer_text(&entry_ref.local_name), "ping");
    assert_eq!(read_owned_buffer_text(&entry_ref.root_name), " ROOT ");
    assert_eq!(
        read_owned_buffer_text(&entry_ref.description),
        "Ping entry."
    );
    assert_eq!(entry_ref.parameters_len, 1);
    let parameter_ref = unsafe { &*entry_ref.parameters };
    assert_eq!(read_owned_buffer_text(&parameter_ref.name), "note");
    assert_eq!(read_owned_buffer_text(&parameter_ref.param_type), "string");
    assert_eq!(
        read_owned_buffer_text(&parameter_ref.description),
        "Optional note."
    );
    assert_eq!(parameter_ref.required, 0);

    unsafe { luaskills_ffi_entry_list_free(entries_out) };

    entries_out = ptr::null_mut();
    list_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let delegated_list_status = unsafe {
        luaskills_ffi_list_entries(
            engine_id,
            FFI_SKILL_AUTHORITY_DELEGATED_TOOL,
            &mut entries_out,
            &mut list_error,
        )
    };
    assert_eq!(delegated_list_status, FFI_STATUS_OK);
    assert!(list_error.ptr.is_null());
    assert!(!entries_out.is_null());
    let delegated_entries_ref = unsafe { &*entries_out };
    assert_eq!(delegated_entries_ref.len, 0);
    unsafe { luaskills_ffi_entry_list_free(entries_out) };

    let mut is_skill_out = 0_u8;
    let mut is_skill_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let delegated_is_skill_status = unsafe {
        luaskills_ffi_is_skill(
            engine_id,
            FFI_SKILL_AUTHORITY_DELEGATED_TOOL,
            tool_name.as_ptr(),
            &mut is_skill_out,
            &mut is_skill_error,
        )
    };
    assert_eq!(delegated_is_skill_status, FFI_STATUS_OK);
    assert!(is_skill_error.ptr.is_null());
    assert_eq!(is_skill_out, 0);

    let mut skill_id_out = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let mut skill_name_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let delegated_skill_name_status = unsafe {
        luaskills_ffi_skill_name_for_tool(
            engine_id,
            FFI_SKILL_AUTHORITY_DELEGATED_TOOL,
            tool_name.as_ptr(),
            &mut skill_id_out,
            &mut skill_name_error,
        )
    };
    assert_eq!(delegated_skill_name_status, FFI_STATUS_OK);
    assert!(skill_name_error.ptr.is_null());
    assert_eq!(read_owned_buffer_text(&skill_id_out), "");
    unsafe { luaskills_ffi_buffer_free(skill_id_out) };

    let (call_args_storage, call_args_buffer) = make_borrowed_buffer("{}");
    let (run_args_storage, run_args_buffer) = make_borrowed_buffer("{}");
    let mut call_result_out: *mut FfiRuntimeInvocationResult = ptr::null_mut();
    let mut call_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let call_status = unsafe {
        luaskills_ffi_call_skill(
            engine_id,
            tool_name.as_ptr(),
            call_args_buffer,
            ptr::null(),
            &mut call_result_out,
            &mut call_error,
        )
    };
    assert_eq!(call_status, FFI_STATUS_OK);
    assert!(call_error.ptr.is_null());
    assert!(!call_result_out.is_null());
    let call_result_ref = unsafe { &*call_result_out };
    assert_eq!(read_owned_buffer_text(&call_result_ref.content), "ok");
    unsafe { luaskills_ffi_invocation_result_free(call_result_out) };

    let run_code =
        CString::new("return vulcan.call('demo-skill-ping', {})").expect("run code cstring");
    let mut run_out = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let mut run_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let run_status = unsafe {
        luaskills_ffi_run_lua(
            engine_id,
            run_code.as_ptr(),
            run_args_buffer,
            ptr::null(),
            &mut run_out,
            &mut run_error,
        )
    };
    assert_eq!(run_status, FFI_STATUS_OK);
    assert!(run_error.ptr.is_null());
    assert_eq!(read_owned_buffer_text(&run_out), "\"ok\"");
    unsafe { luaskills_ffi_buffer_free(run_out) };
    let _ = (call_args_storage, run_args_storage);

    let prompt_name = CString::new("demo").expect("prompt name cstring");
    let argument_name = CString::new("target").expect("argument name cstring");
    let mut prompt_values_out: *mut FfiStringArray = ptr::null_mut();
    let mut prompt_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let prompt_status = unsafe {
        luaskills_ffi_prompt_argument_completions(
            engine_id,
            FFI_SKILL_AUTHORITY_DELEGATED_TOOL,
            prompt_name.as_ptr(),
            argument_name.as_ptr(),
            &mut prompt_values_out,
            &mut prompt_error,
        )
    };
    assert_eq!(prompt_status, FFI_STATUS_OK);
    assert!(prompt_error.ptr.is_null());
    assert!(!prompt_values_out.is_null());
    let prompt_values_ref = unsafe { &*prompt_values_out };
    assert_eq!(prompt_values_ref.len, 0);
    unsafe { luaskills_ffi_string_array_free(prompt_values_out) };

    let mut free_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let free_status = unsafe { luaskills_ffi_engine_free(engine_id, &mut free_error) };
    assert_eq!(free_status, FFI_STATUS_OK);
    assert!(free_error.ptr.is_null());

    let _ = std::fs::remove_dir_all(&temp_root);
}

/// Verify standard call_skill accepts borrowed JSON buffers for args and invocation context.
/// 验证标准 call_skill 会接受作为 args 与调用上下文输入的借用 JSON 缓冲。
#[test]
fn standard_ffi_call_skill_accepts_borrowed_json_buffers() {
    let temp_root = std::env::temp_dir().join(format!(
        "luaskills_standard_ffi_callskill_test_{}",
        std::process::id()
    ));
    if temp_root.exists() {
        let _ = std::fs::remove_dir_all(&temp_root);
    }

    let root_skills_root = temp_root.join("root_skills");
    let skills_root = temp_root.join("skills");
    let skill_dir = skills_root.join("demo-skill");
    std::fs::create_dir_all(&root_skills_root).expect("create root skills root");
    std::fs::create_dir_all(skill_dir.join("runtime")).expect("create runtime directory");
    std::fs::create_dir_all(temp_root.join("temp")).expect("create temp directory");
    std::fs::create_dir_all(temp_root.join("resources")).expect("create resources directory");
    std::fs::create_dir_all(temp_root.join("lua_packages")).expect("create lua_packages directory");
    std::fs::create_dir_all(temp_root.join("bin").join("tools")).expect("create tools directory");
    std::fs::create_dir_all(temp_root.join("libs")).expect("create libs directory");
    std::fs::write(
            skill_dir.join("skill.yaml"),
            "name: demo-skill\nversion: 0.1.0\nenable: true\nentries:\n  - name: ping\n    description: Ping entry.\n    lua_entry: runtime/ping.lua\n    lua_module: demo_skill_ping\n    parameters:\n      - name: note\n        type: string\n        description: Optional note.\n        required: false\n",
        )
        .expect("write skill yaml");
    std::fs::write(
            skill_dir.join("runtime").join("ping.lua"),
            "return function(args)\n  local note = ''\n  if type(args) == 'table' and type(args.note) == 'string' then\n    note = args.note\n  end\n  if note ~= '' then\n    return 'standard-ffi-test:' .. note\n  end\n  return 'standard-ffi-test:ok'\nend\n",
        )
        .expect("write runtime lua");

    let temp_dir_text =
        CString::new(temp_root.join("temp").display().to_string()).expect("temp_dir cstring");
    let resources_dir_text = CString::new(temp_root.join("resources").display().to_string())
        .expect("resources_dir cstring");
    let lua_packages_dir_text = CString::new(temp_root.join("lua_packages").display().to_string())
        .expect("lua_packages_dir cstring");
    let tool_root_dir_text =
        CString::new(temp_root.join("bin").join("tools").display().to_string())
            .expect("tool_root_dir cstring");
    let ffi_root_dir_text =
        CString::new(temp_root.join("libs").display().to_string()).expect("ffi_root cstring");
    let dependency_dir_name = CString::new("dependencies").expect("dependencies cstring");
    let state_dir_name = CString::new("state").expect("state cstring");
    let database_dir_name = CString::new("databases").expect("databases cstring");
    let root_name = CString::new("ROOT").expect("root name cstring");
    let skills_root_text =
        CString::new(skills_root.display().to_string()).expect("skills root cstring");
    let tool_name = CString::new("demo-skill-ping").expect("tool name cstring");

    let host_options = FfiLuaRuntimeHostOptions {
        temp_dir: temp_dir_text.as_ptr(),
        resources_dir: resources_dir_text.as_ptr(),
        lua_packages_dir: lua_packages_dir_text.as_ptr(),
        host_provided_tool_root: tool_root_dir_text.as_ptr(),
        host_provided_lua_root: lua_packages_dir_text.as_ptr(),
        host_provided_ffi_root: ffi_root_dir_text.as_ptr(),
        system_lua_lib_dir: ptr::null(),
        download_cache_root: ptr::null(),
        dependency_dir_name: dependency_dir_name.as_ptr(),
        state_dir_name: state_dir_name.as_ptr(),
        database_dir_name: database_dir_name.as_ptr(),
        skill_config_file_path: ptr::null(),
        allow_network_download: 0,
        github_base_url: ptr::null(),
        github_api_base_url: ptr::null(),
        official_skill_hub_base_url: ptr::null(),
        enable_private_url_skill_install: 0,
        private_skill_source_allowlist: ptr::null(),
        private_skill_source_allowlist_len: 0,
        sqlite_library_path: ptr::null(),
        sqlite_provider_mode: FFI_PROVIDER_MODE_DYNAMIC_LIBRARY,
        sqlite_callback_mode: FFI_CALLBACK_MODE_STANDARD,
        lancedb_library_path: ptr::null(),
        lancedb_provider_mode: FFI_PROVIDER_MODE_DYNAMIC_LIBRARY,
        lancedb_callback_mode: FFI_CALLBACK_MODE_STANDARD,
        space_controller_endpoint: ptr::null(),
        space_controller_auto_spawn: 0,
        space_controller_executable_path: ptr::null(),
        space_controller_process_mode: FFI_SPACE_CONTROLLER_PROCESS_MODE_SERVICE,
        cache_config: ptr::null(),
        runlua_pool_config: ptr::null(),
        reserved_entry_names: ptr::null(),
        reserved_entry_names_len: 0,
        ignored_skill_ids: ptr::null(),
        ignored_skill_ids_len: 0,
        enable_skill_management_bridge: 0,
        default_text_encoding: ptr::null(),
        disable_managed_io_compat: 0,
    };
    let engine_options = FfiLuaEngineOptions {
        pool: FfiLuaVmPoolConfig {
            min_size: 1,
            max_size: 1,
            idle_ttl_secs: 30,
        },
        host: host_options,
    };

    let mut engine_id = 0_u64;
    let mut error_out = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let engine_status =
        unsafe { luaskills_ffi_engine_new(&engine_options, &mut engine_id, &mut error_out) };
    assert_eq!(engine_status, FFI_STATUS_OK);
    assert!(error_out.ptr.is_null());

    let ffi_skill_roots = [FfiRuntimeSkillRoot {
        name: root_name.as_ptr(),
        skills_dir: skills_root_text.as_ptr(),
    }];
    let mut load_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let load_status = unsafe {
        luaskills_ffi_load_from_roots(
            engine_id,
            ffi_skill_roots.as_ptr(),
            ffi_skill_roots.len(),
            &mut load_error,
        )
    };
    assert_eq!(load_status, FFI_STATUS_OK);
    assert!(load_error.ptr.is_null());

    let (_args_storage, args_buffer) = make_borrowed_buffer(r#"{"note":"ffi"}"#);
    let (_request_storage, request_buffer) =
        make_borrowed_buffer(r#"{"transport_name":"ffi-test"}"#);
    let (_budget_storage, budget_buffer) = make_borrowed_buffer(r#"{"budget":7}"#);
    let (_tool_storage, tool_buffer) = make_borrowed_buffer(r#"{"mode":"demo-mode"}"#);
    let invocation_context = FfiLuaInvocationContext {
        request_context_json: request_buffer,
        client_budget_json: budget_buffer,
        tool_config_json: tool_buffer,
    };

    let mut result_out: *mut FfiRuntimeInvocationResult = ptr::null_mut();
    let mut call_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let call_status = unsafe {
        luaskills_ffi_call_skill(
            engine_id,
            tool_name.as_ptr(),
            args_buffer,
            &invocation_context,
            &mut result_out,
            &mut call_error,
        )
    };
    assert_eq!(call_status, FFI_STATUS_OK);
    assert!(call_error.ptr.is_null());
    assert!(!result_out.is_null());

    let result_ref = unsafe { &*result_out };
    assert_eq!(
        read_owned_buffer_text(&result_ref.content),
        "standard-ffi-test:ffi"
    );
    assert_eq!(result_ref.content_lines, 1);
    unsafe { luaskills_ffi_invocation_result_free(result_out) };

    let mut free_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let free_status = unsafe { luaskills_ffi_engine_free(engine_id, &mut free_error) };
    assert_eq!(free_status, FFI_STATUS_OK);
    assert!(free_error.ptr.is_null());

    let _ = std::fs::remove_dir_all(&temp_root);
}

/// Verify standard run_lua accepts borrowed JSON buffers for args and invocation context.
/// 验证标准 run_lua 会接受作为 args 与调用上下文输入的借用 JSON 缓冲。
#[test]
fn standard_ffi_run_lua_accepts_borrowed_json_buffers() {
    let temp_root = std::env::temp_dir().join(format!(
        "luaskills_standard_ffi_runlua_test_{}",
        std::process::id()
    ));
    if temp_root.exists() {
        let _ = std::fs::remove_dir_all(&temp_root);
    }

    std::fs::create_dir_all(temp_root.join("temp")).expect("create temp directory");
    std::fs::create_dir_all(temp_root.join("resources")).expect("create resources directory");
    std::fs::create_dir_all(temp_root.join("lua_packages")).expect("create lua_packages directory");
    std::fs::create_dir_all(temp_root.join("bin").join("tools")).expect("create tools directory");
    std::fs::create_dir_all(temp_root.join("libs")).expect("create libs directory");

    let temp_dir_text =
        CString::new(temp_root.join("temp").display().to_string()).expect("temp_dir cstring");
    let resources_dir_text = CString::new(temp_root.join("resources").display().to_string())
        .expect("resources_dir cstring");
    let lua_packages_dir_text = CString::new(temp_root.join("lua_packages").display().to_string())
        .expect("lua_packages_dir cstring");
    let tool_root_dir_text =
        CString::new(temp_root.join("bin").join("tools").display().to_string())
            .expect("tool_root_dir cstring");
    let ffi_root_dir_text =
        CString::new(temp_root.join("libs").display().to_string()).expect("ffi_root cstring");
    let dependency_dir_name = CString::new("dependencies").expect("dependencies cstring");
    let state_dir_name = CString::new("state").expect("state cstring");
    let database_dir_name = CString::new("databases").expect("databases cstring");

    let host_options = FfiLuaRuntimeHostOptions {
        temp_dir: temp_dir_text.as_ptr(),
        resources_dir: resources_dir_text.as_ptr(),
        lua_packages_dir: lua_packages_dir_text.as_ptr(),
        host_provided_tool_root: tool_root_dir_text.as_ptr(),
        host_provided_lua_root: lua_packages_dir_text.as_ptr(),
        host_provided_ffi_root: ffi_root_dir_text.as_ptr(),
        system_lua_lib_dir: ptr::null(),
        download_cache_root: ptr::null(),
        dependency_dir_name: dependency_dir_name.as_ptr(),
        state_dir_name: state_dir_name.as_ptr(),
        database_dir_name: database_dir_name.as_ptr(),
        skill_config_file_path: ptr::null(),
        allow_network_download: 0,
        github_base_url: ptr::null(),
        github_api_base_url: ptr::null(),
        official_skill_hub_base_url: ptr::null(),
        enable_private_url_skill_install: 0,
        private_skill_source_allowlist: ptr::null(),
        private_skill_source_allowlist_len: 0,
        sqlite_library_path: ptr::null(),
        sqlite_provider_mode: FFI_PROVIDER_MODE_DYNAMIC_LIBRARY,
        sqlite_callback_mode: FFI_CALLBACK_MODE_STANDARD,
        lancedb_library_path: ptr::null(),
        lancedb_provider_mode: FFI_PROVIDER_MODE_DYNAMIC_LIBRARY,
        lancedb_callback_mode: FFI_CALLBACK_MODE_STANDARD,
        space_controller_endpoint: ptr::null(),
        space_controller_auto_spawn: 0,
        space_controller_executable_path: ptr::null(),
        space_controller_process_mode: FFI_SPACE_CONTROLLER_PROCESS_MODE_SERVICE,
        cache_config: ptr::null(),
        runlua_pool_config: ptr::null(),
        reserved_entry_names: ptr::null(),
        reserved_entry_names_len: 0,
        ignored_skill_ids: ptr::null(),
        ignored_skill_ids_len: 0,
        enable_skill_management_bridge: 0,
        default_text_encoding: ptr::null(),
        disable_managed_io_compat: 0,
    };
    let engine_options = FfiLuaEngineOptions {
        pool: FfiLuaVmPoolConfig {
            min_size: 1,
            max_size: 1,
            idle_ttl_secs: 30,
        },
        host: host_options,
    };

    let mut engine_id = 0_u64;
    let mut error_out = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let engine_status =
        unsafe { luaskills_ffi_engine_new(&engine_options, &mut engine_id, &mut error_out) };
    assert_eq!(engine_status, FFI_STATUS_OK);
    assert!(error_out.ptr.is_null());

    let code =
            CString::new("return { note = args.note, transport = vulcan.context.request.transport_name, budget = vulcan.context.client_budget.budget, mode = vulcan.context.tool_config.mode }")
                .expect("code cstring");
    let (_args_storage, args_buffer) = make_borrowed_buffer(r#"{"note":"demo"}"#);
    let (_request_storage, request_buffer) =
        make_borrowed_buffer(r#"{"transport_name":"ffi-test"}"#);
    let (_budget_storage, budget_buffer) = make_borrowed_buffer(r#"{"budget":7}"#);
    let (_tool_storage, tool_buffer) = make_borrowed_buffer(r#"{"mode":"demo-mode"}"#);
    let invocation_context = FfiLuaInvocationContext {
        request_context_json: request_buffer,
        client_budget_json: budget_buffer,
        tool_config_json: tool_buffer,
    };

    let mut result_json_out = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let mut run_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let run_status = unsafe {
        luaskills_ffi_run_lua(
            engine_id,
            code.as_ptr(),
            args_buffer,
            &invocation_context,
            &mut result_json_out,
            &mut run_error,
        )
    };
    assert_eq!(run_status, FFI_STATUS_OK);
    assert!(run_error.ptr.is_null());

    let result_json_text = read_owned_buffer_text(&result_json_out);
    let result_json: Value =
        serde_json::from_str(&result_json_text).expect("run_lua result must be valid json");
    assert_eq!(result_json["note"], "demo");
    assert_eq!(result_json["transport"], "ffi-test");
    assert_eq!(result_json["budget"], 7);
    assert_eq!(result_json["mode"], "demo-mode");
    unsafe { luaskills_ffi_buffer_free(result_json_out) };

    let mut free_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let free_status = unsafe { luaskills_ffi_engine_free(engine_id, &mut free_error) };
    assert_eq!(free_status, FFI_STATUS_OK);
    assert!(free_error.ptr.is_null());

    let _ = std::fs::remove_dir_all(&temp_root);
}

/// Verify the standard C ABI skill-config helpers support one full set/get/list/delete roundtrip.
/// 验证标准 C ABI 的技能配置辅助接口支持完整的 set/get/list/delete 往返流程。
#[test]
fn standard_ffi_skill_config_round_trip() {
    let temp_root = std::env::temp_dir().join(format!(
        "luaskills_standard_ffi_skill_config_test_{}",
        std::process::id()
    ));
    if temp_root.exists() {
        let _ = std::fs::remove_dir_all(&temp_root);
    }

    std::fs::create_dir_all(temp_root.join("temp")).expect("create temp directory");
    std::fs::create_dir_all(temp_root.join("resources")).expect("create resources directory");
    std::fs::create_dir_all(temp_root.join("lua_packages")).expect("create lua_packages directory");
    std::fs::create_dir_all(temp_root.join("bin").join("tools")).expect("create tools directory");
    std::fs::create_dir_all(temp_root.join("libs")).expect("create libs directory");

    let temp_dir_text =
        CString::new(temp_root.join("temp").display().to_string()).expect("temp_dir cstring");
    let resources_dir_text = CString::new(temp_root.join("resources").display().to_string())
        .expect("resources_dir cstring");
    let lua_packages_dir_text = CString::new(temp_root.join("lua_packages").display().to_string())
        .expect("lua_packages_dir cstring");
    let tool_root_dir_text =
        CString::new(temp_root.join("bin").join("tools").display().to_string())
            .expect("tool_root_dir cstring");
    let ffi_root_dir_text =
        CString::new(temp_root.join("libs").display().to_string()).expect("ffi_root cstring");
    let dependency_dir_name = CString::new("dependencies").expect("dependencies cstring");
    let state_dir_name = CString::new("state").expect("state cstring");
    let database_dir_name = CString::new("databases").expect("databases cstring");
    let skill_config_file_path = CString::new(
        temp_root
            .join("config")
            .join("skill_config.json")
            .display()
            .to_string(),
    )
    .expect("skill config file path cstring");
    let skill_id = CString::new("demo-skill").expect("skill_id cstring");
    let key = CString::new("api_token").expect("key cstring");
    let value = CString::new("sk-standard-ffi").expect("value cstring");

    let host_options = FfiLuaRuntimeHostOptions {
        temp_dir: temp_dir_text.as_ptr(),
        resources_dir: resources_dir_text.as_ptr(),
        lua_packages_dir: lua_packages_dir_text.as_ptr(),
        host_provided_tool_root: tool_root_dir_text.as_ptr(),
        host_provided_lua_root: lua_packages_dir_text.as_ptr(),
        host_provided_ffi_root: ffi_root_dir_text.as_ptr(),
        system_lua_lib_dir: ptr::null(),
        download_cache_root: ptr::null(),
        dependency_dir_name: dependency_dir_name.as_ptr(),
        state_dir_name: state_dir_name.as_ptr(),
        database_dir_name: database_dir_name.as_ptr(),
        skill_config_file_path: skill_config_file_path.as_ptr(),
        allow_network_download: 0,
        github_base_url: ptr::null(),
        github_api_base_url: ptr::null(),
        official_skill_hub_base_url: ptr::null(),
        enable_private_url_skill_install: 0,
        private_skill_source_allowlist: ptr::null(),
        private_skill_source_allowlist_len: 0,
        sqlite_library_path: ptr::null(),
        sqlite_provider_mode: FFI_PROVIDER_MODE_DYNAMIC_LIBRARY,
        sqlite_callback_mode: FFI_CALLBACK_MODE_STANDARD,
        lancedb_library_path: ptr::null(),
        lancedb_provider_mode: FFI_PROVIDER_MODE_DYNAMIC_LIBRARY,
        lancedb_callback_mode: FFI_CALLBACK_MODE_STANDARD,
        space_controller_endpoint: ptr::null(),
        space_controller_auto_spawn: 0,
        space_controller_executable_path: ptr::null(),
        space_controller_process_mode: FFI_SPACE_CONTROLLER_PROCESS_MODE_SERVICE,
        cache_config: ptr::null(),
        runlua_pool_config: ptr::null(),
        reserved_entry_names: ptr::null(),
        reserved_entry_names_len: 0,
        ignored_skill_ids: ptr::null(),
        ignored_skill_ids_len: 0,
        enable_skill_management_bridge: 0,
        default_text_encoding: ptr::null(),
        disable_managed_io_compat: 0,
    };
    let engine_options = FfiLuaEngineOptions {
        pool: FfiLuaVmPoolConfig {
            min_size: 1,
            max_size: 1,
            idle_ttl_secs: 30,
        },
        host: host_options,
    };

    let mut engine_id = 0_u64;
    let mut error_out = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let engine_status =
        unsafe { luaskills_ffi_engine_new(&engine_options, &mut engine_id, &mut error_out) };
    assert_eq!(engine_status, FFI_STATUS_OK);
    assert!(error_out.ptr.is_null());

    let mut set_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let set_status = unsafe {
        luaskills_ffi_skill_config_set(
            engine_id,
            skill_id.as_ptr(),
            key.as_ptr(),
            value.as_ptr(),
            &mut set_error,
        )
    };
    assert_eq!(set_status, FFI_STATUS_OK);
    assert!(set_error.ptr.is_null());

    let mut value_out = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let mut found_out = 0_u8;
    let mut get_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let get_status = unsafe {
        luaskills_ffi_skill_config_get(
            engine_id,
            skill_id.as_ptr(),
            key.as_ptr(),
            &mut value_out,
            &mut found_out,
            &mut get_error,
        )
    };
    assert_eq!(get_status, FFI_STATUS_OK);
    assert!(get_error.ptr.is_null());
    assert_eq!(found_out, 1);
    assert_eq!(read_owned_buffer_text(&value_out), "sk-standard-ffi");
    unsafe { luaskills_ffi_buffer_free(value_out) };

    let empty_value = CString::new("").expect("empty value cstring");
    let mut empty_set_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let empty_set_status = unsafe {
        luaskills_ffi_skill_config_set(
            engine_id,
            skill_id.as_ptr(),
            key.as_ptr(),
            empty_value.as_ptr(),
            &mut empty_set_error,
        )
    };
    assert_eq!(empty_set_status, FFI_STATUS_OK);
    assert!(empty_set_error.ptr.is_null());

    let mut empty_value_out = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let mut empty_found_out = 0_u8;
    let mut empty_get_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let empty_get_status = unsafe {
        luaskills_ffi_skill_config_get(
            engine_id,
            skill_id.as_ptr(),
            key.as_ptr(),
            &mut empty_value_out,
            &mut empty_found_out,
            &mut empty_get_error,
        )
    };
    assert_eq!(empty_get_status, FFI_STATUS_OK);
    assert!(empty_get_error.ptr.is_null());
    assert_eq!(empty_found_out, 1);
    assert_eq!(read_owned_buffer_text(&empty_value_out), "");
    unsafe { luaskills_ffi_buffer_free(empty_value_out) };

    let mut list_out = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let mut list_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let list_status = unsafe {
        luaskills_ffi_skill_config_list(
            engine_id,
            skill_id.as_ptr(),
            &mut list_out,
            &mut list_error,
        )
    };
    assert_eq!(list_status, FFI_STATUS_OK);
    assert!(list_error.ptr.is_null());
    let list_json: serde_json::Value = serde_json::from_str(&read_owned_buffer_text(&list_out))
        .expect("skill config list json should parse");
    assert_eq!(list_json.as_array().map(Vec::len), Some(1));
    assert_eq!(list_json[0]["skill_id"], "demo-skill");
    assert_eq!(list_json[0]["key"], "api_token");
    assert_eq!(list_json[0]["value"], "");
    unsafe { luaskills_ffi_buffer_free(list_out) };

    let mut deleted_out = 0_u8;
    let mut delete_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let delete_status = unsafe {
        luaskills_ffi_skill_config_delete(
            engine_id,
            skill_id.as_ptr(),
            key.as_ptr(),
            &mut deleted_out,
            &mut delete_error,
        )
    };
    assert_eq!(delete_status, FFI_STATUS_OK);
    assert!(delete_error.ptr.is_null());
    assert_eq!(deleted_out, 1);

    let mut free_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let free_status = unsafe { luaskills_ffi_engine_free(engine_id, &mut free_error) };
    assert_eq!(free_status, FFI_STATUS_OK);
    assert!(free_error.ptr.is_null());

    let _ = std::fs::remove_dir_all(&temp_root);
}

/// Verify standard disable/enable lifecycle calls update the runtime view in place.
/// 验证标准 disable/enable 生命周期调用会原地更新运行时视图。
#[test]
fn standard_ffi_disable_and_enable_skill_round_trip() {
    let temp_root = std::env::temp_dir().join(format!(
        "luaskills_standard_ffi_lifecycle_test_{}",
        std::process::id()
    ));
    if temp_root.exists() {
        let _ = std::fs::remove_dir_all(&temp_root);
    }

    let root_skills_root = temp_root.join("root_skills");
    let skills_root = temp_root.join("skills");
    let skill_dir = skills_root.join("demo-skill");
    std::fs::create_dir_all(&root_skills_root).expect("create root skills root");
    std::fs::create_dir_all(skill_dir.join("runtime")).expect("create runtime directory");
    std::fs::create_dir_all(temp_root.join("temp")).expect("create temp directory");
    std::fs::create_dir_all(temp_root.join("resources")).expect("create resources directory");
    std::fs::create_dir_all(temp_root.join("lua_packages")).expect("create lua_packages directory");
    std::fs::create_dir_all(temp_root.join("bin").join("tools")).expect("create tools directory");
    std::fs::create_dir_all(temp_root.join("libs")).expect("create libs directory");
    std::fs::write(
            skill_dir.join("skill.yaml"),
            "name: demo-skill\nversion: 0.1.0\nenable: true\nentries:\n  - name: ping\n    description: Ping entry.\n    lua_entry: runtime/ping.lua\n    lua_module: demo_skill_ping\n    parameters:\n      - name: note\n        type: string\n        description: Optional note.\n        required: false\n",
        )
        .expect("write skill yaml");
    std::fs::write(
            skill_dir.join("runtime").join("ping.lua"),
            "return function(args)\n  local note = ''\n  if type(args) == 'table' and type(args.note) == 'string' then\n    note = args.note\n  end\n  if note ~= '' then\n    return 'lifecycle:' .. note\n  end\n  return 'lifecycle:ok'\nend\n",
        )
        .expect("write runtime lua");

    let temp_dir_text =
        CString::new(temp_root.join("temp").display().to_string()).expect("temp_dir cstring");
    let resources_dir_text = CString::new(temp_root.join("resources").display().to_string())
        .expect("resources_dir cstring");
    let lua_packages_dir_text = CString::new(temp_root.join("lua_packages").display().to_string())
        .expect("lua_packages_dir cstring");
    let tool_root_dir_text =
        CString::new(temp_root.join("bin").join("tools").display().to_string())
            .expect("tool_root_dir cstring");
    let ffi_root_dir_text =
        CString::new(temp_root.join("libs").display().to_string()).expect("ffi_root cstring");
    let dependency_dir_name = CString::new("dependencies").expect("dependencies cstring");
    let state_dir_name = CString::new("state").expect("state cstring");
    let database_dir_name = CString::new("databases").expect("databases cstring");
    let root_name = CString::new("ROOT").expect("root name cstring");
    let user_name = CString::new("USER").expect("user name cstring");
    let root_skills_root_text =
        CString::new(root_skills_root.display().to_string()).expect("root skills cstring");
    let skills_root_text =
        CString::new(skills_root.display().to_string()).expect("skills root cstring");
    let skill_id = CString::new("demo-skill").expect("skill_id cstring");
    let tool_name = CString::new("demo-skill-ping").expect("tool_name cstring");
    let disable_reason = CString::new("maintenance").expect("disable reason cstring");

    let host_options = FfiLuaRuntimeHostOptions {
        temp_dir: temp_dir_text.as_ptr(),
        resources_dir: resources_dir_text.as_ptr(),
        lua_packages_dir: lua_packages_dir_text.as_ptr(),
        host_provided_tool_root: tool_root_dir_text.as_ptr(),
        host_provided_lua_root: lua_packages_dir_text.as_ptr(),
        host_provided_ffi_root: ffi_root_dir_text.as_ptr(),
        system_lua_lib_dir: ptr::null(),
        download_cache_root: ptr::null(),
        dependency_dir_name: dependency_dir_name.as_ptr(),
        state_dir_name: state_dir_name.as_ptr(),
        database_dir_name: database_dir_name.as_ptr(),
        skill_config_file_path: ptr::null(),
        allow_network_download: 0,
        github_base_url: ptr::null(),
        github_api_base_url: ptr::null(),
        official_skill_hub_base_url: ptr::null(),
        enable_private_url_skill_install: 0,
        private_skill_source_allowlist: ptr::null(),
        private_skill_source_allowlist_len: 0,
        sqlite_library_path: ptr::null(),
        sqlite_provider_mode: FFI_PROVIDER_MODE_DYNAMIC_LIBRARY,
        sqlite_callback_mode: FFI_CALLBACK_MODE_STANDARD,
        lancedb_library_path: ptr::null(),
        lancedb_provider_mode: FFI_PROVIDER_MODE_DYNAMIC_LIBRARY,
        lancedb_callback_mode: FFI_CALLBACK_MODE_STANDARD,
        space_controller_endpoint: ptr::null(),
        space_controller_auto_spawn: 0,
        space_controller_executable_path: ptr::null(),
        space_controller_process_mode: FFI_SPACE_CONTROLLER_PROCESS_MODE_SERVICE,
        cache_config: ptr::null(),
        runlua_pool_config: ptr::null(),
        reserved_entry_names: ptr::null(),
        reserved_entry_names_len: 0,
        ignored_skill_ids: ptr::null(),
        ignored_skill_ids_len: 0,
        enable_skill_management_bridge: 0,
        default_text_encoding: ptr::null(),
        disable_managed_io_compat: 0,
    };
    let engine_options = FfiLuaEngineOptions {
        pool: FfiLuaVmPoolConfig {
            min_size: 1,
            max_size: 1,
            idle_ttl_secs: 30,
        },
        host: host_options,
    };

    let mut engine_id = 0_u64;
    let mut error_out = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let engine_status =
        unsafe { luaskills_ffi_engine_new(&engine_options, &mut engine_id, &mut error_out) };
    assert_eq!(engine_status, FFI_STATUS_OK);
    assert!(error_out.ptr.is_null());

    let ffi_skill_roots = [
        FfiRuntimeSkillRoot {
            name: root_name.as_ptr(),
            skills_dir: root_skills_root_text.as_ptr(),
        },
        FfiRuntimeSkillRoot {
            name: user_name.as_ptr(),
            skills_dir: skills_root_text.as_ptr(),
        },
    ];

    let mut load_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let load_status = unsafe {
        luaskills_ffi_load_from_roots(
            engine_id,
            ffi_skill_roots.as_ptr(),
            ffi_skill_roots.len(),
            &mut load_error,
        )
    };
    assert_eq!(load_status, FFI_STATUS_OK);
    assert!(load_error.ptr.is_null());

    let mut entries_out: *mut FfiRuntimeEntryDescriptorList = ptr::null_mut();
    let mut list_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let list_status = unsafe {
        luaskills_ffi_list_entries(
            engine_id,
            FFI_SKILL_AUTHORITY_SYSTEM,
            &mut entries_out,
            &mut list_error,
        )
    };
    assert_eq!(list_status, FFI_STATUS_OK);
    assert!(list_error.ptr.is_null());
    assert!(!entries_out.is_null());
    let entries_ref = unsafe { &*entries_out };
    assert_eq!(entries_ref.len, 1);
    unsafe { luaskills_ffi_entry_list_free(entries_out) };

    let (_before_disable_args_storage, before_disable_args_buffer) =
        make_borrowed_buffer(r#"{"note":"before-disable"}"#);
    let mut result_out: *mut FfiRuntimeInvocationResult = ptr::null_mut();
    let mut call_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let call_status = unsafe {
        luaskills_ffi_call_skill(
            engine_id,
            tool_name.as_ptr(),
            before_disable_args_buffer,
            ptr::null(),
            &mut result_out,
            &mut call_error,
        )
    };
    assert_eq!(call_status, FFI_STATUS_OK);
    assert!(call_error.ptr.is_null());
    let result_ref = unsafe { &*result_out };
    assert_eq!(
        read_owned_buffer_text(&result_ref.content),
        "lifecycle:before-disable"
    );
    unsafe { luaskills_ffi_invocation_result_free(result_out) };

    let mut disable_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let disable_status = unsafe {
        luaskills_ffi_disable_skill(
            engine_id,
            ffi_skill_roots.as_ptr(),
            ffi_skill_roots.len(),
            skill_id.as_ptr(),
            disable_reason.as_ptr(),
            &mut disable_error,
        )
    };
    assert_eq!(disable_status, FFI_STATUS_OK);
    assert!(disable_error.ptr.is_null());

    entries_out = ptr::null_mut();
    list_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let disabled_list_status = unsafe {
        luaskills_ffi_list_entries(
            engine_id,
            FFI_SKILL_AUTHORITY_SYSTEM,
            &mut entries_out,
            &mut list_error,
        )
    };
    assert_eq!(disabled_list_status, FFI_STATUS_OK);
    assert!(list_error.ptr.is_null());
    assert!(!entries_out.is_null());
    let disabled_entries_ref = unsafe { &*entries_out };
    assert_eq!(disabled_entries_ref.len, 0);
    unsafe { luaskills_ffi_entry_list_free(entries_out) };

    result_out = ptr::null_mut();
    call_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let (_disabled_args_storage, disabled_args_buffer) =
        make_borrowed_buffer(r#"{"note":"before-disable"}"#);
    let disabled_call_status = unsafe {
        luaskills_ffi_call_skill(
            engine_id,
            tool_name.as_ptr(),
            disabled_args_buffer,
            ptr::null(),
            &mut result_out,
            &mut call_error,
        )
    };
    assert_ne!(disabled_call_status, FFI_STATUS_OK);
    assert!(result_out.is_null());
    assert!(!call_error.ptr.is_null());
    unsafe { luaskills_ffi_buffer_free(call_error) };

    let mut enable_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let enable_status = unsafe {
        luaskills_ffi_enable_skill(
            engine_id,
            ffi_skill_roots.as_ptr(),
            ffi_skill_roots.len(),
            skill_id.as_ptr(),
            &mut enable_error,
        )
    };
    assert_eq!(enable_status, FFI_STATUS_OK);
    assert!(enable_error.ptr.is_null());

    entries_out = ptr::null_mut();
    list_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let enabled_list_status = unsafe {
        luaskills_ffi_list_entries(
            engine_id,
            FFI_SKILL_AUTHORITY_SYSTEM,
            &mut entries_out,
            &mut list_error,
        )
    };
    assert_eq!(enabled_list_status, FFI_STATUS_OK);
    assert!(list_error.ptr.is_null());
    assert!(!entries_out.is_null());
    let enabled_entries_ref = unsafe { &*entries_out };
    assert_eq!(enabled_entries_ref.len, 1);
    unsafe { luaskills_ffi_entry_list_free(entries_out) };

    let (_enabled_args_storage, enabled_args_buffer) =
        make_borrowed_buffer(r#"{"note":"after-enable"}"#);
    result_out = ptr::null_mut();
    call_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let enabled_call_status = unsafe {
        luaskills_ffi_call_skill(
            engine_id,
            tool_name.as_ptr(),
            enabled_args_buffer,
            ptr::null(),
            &mut result_out,
            &mut call_error,
        )
    };
    assert_eq!(enabled_call_status, FFI_STATUS_OK);
    assert!(call_error.ptr.is_null());
    let enabled_result_ref = unsafe { &*result_out };
    assert_eq!(
        read_owned_buffer_text(&enabled_result_ref.content),
        "lifecycle:after-enable"
    );
    unsafe { luaskills_ffi_invocation_result_free(result_out) };

    let mut free_error = FfiOwnedBuffer {
        ptr: ptr::null_mut(),
        len: 0,
    };
    let free_status = unsafe { luaskills_ffi_engine_free(engine_id, &mut free_error) };
    assert_eq!(free_status, FFI_STATUS_OK);
    assert!(free_error.ptr.is_null());

    let _ = std::fs::remove_dir_all(&temp_root);
}
