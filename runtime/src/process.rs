// Copy borrowed native strings before offloading: the OS worker may outlive a
// coroutine suspension, and must own Rust data rather than borrow Foster storage.
// The returned text is copied back into a new owned native String.
#[unsafe(no_mangle)]
extern "C" fn foster_rt_v5_process_exchange(
    operation: i64,
    token: i64,
    executable: usize,
    arguments: usize,
    directory: usize,
    limit: i64,
) -> usize {
    // SAFETY: Generated callers keep all three String owners alive and immutable
    // until these synchronous copies finish; no native handle enters the worker.
    let executable = unsafe { string_value(executable) }.to_owned();
    let arguments = unsafe { string_value(arguments) }.to_owned();
    let directory = unsafe { string_value(directory) }.to_owned();
    let response = foster_host_blocking(move || {
        process_runtime::exchange(operation, token, &executable, &arguments, &directory, limit)
    });
    owned_string(&response)
}

#[unsafe(no_mangle)]
extern "C" fn foster_rt_v5_process_reserve() -> i64 {
    process_runtime::reserve()
}

#[unsafe(no_mangle)]
extern "C" fn foster_rt_v5_process_wait(token: i64) -> usize {
    let response = foster_host_blocking(move || process_runtime::exchange(2, token, "", "", "", 0));
    owned_string(&response)
}

#[unsafe(no_mangle)]
extern "C" fn foster_rt_v5_process_release(token: i64) -> u8 {
    foster_host_blocking(move || process_runtime::release(token));
    0
}
